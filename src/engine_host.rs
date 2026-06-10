// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 phase 01 — the **engine-host kernel natives**: the Rust mechanics behind
//! the kernel's loft library (`lib/engine_host`).  The host-boundary principle
//! ([`plans/18-engine-host/ENGINE_HOST.md`]): this module owns **mechanics** —
//! the socket pump (non-blocking, the phase-00 **peek pattern** from day one),
//! the event queue, drift-free tick scheduling, send/broadcast — and *no game
//! meaning*.  The loft side owns meaning: `run(port, tick_us, on_event, on_tick)`
//! loops over these natives and invokes the user's closures via ordinary fn-ref
//! calls (probe 2: no Rust→closure machinery exists or is needed).
//!
//! Scope: the **events class** (WS text frames, queue-to-empty) from phase 01,
//! plus the phase-05a **state-sync class over UDP** — the quick channel beside
//! the websockets.  Events and bulk STAY on WS (reliable+ordered for free); the
//! UDP side carries ONLY latest-value state, so it needs no retransmit,
//! fragmentation, or congestion machinery:
//!
//! - One UDP socket on the SAME port number as the TCP listener (zero config).
//! - Identity = the datagram source address bound by a **cookie handshake**:
//!   the kernel issues a per-connection cookie (`n_kernel_udp_cookie`); the
//!   loft side transports it inside its own protocol (cookie *issuance* is
//!   mechanics, cookie *transport* is meaning); the client echoes it in a
//!   `H:<cookie>` datagram; the kernel binds the source addr to that cid and
//!   acks `A:<cid>`.
//! - Inbound `S:<seq>:<payload>` datagrams **conflate to newest per sender**
//!   (a higher seq overwrites the slot; stale/reordered seqs are discarded —
//!   never apply an old pose).  The loft side drains the dirty slots at tick
//!   time via `n_kernel_sync_next` — late-latch by construction.
//! - Outbound `n_kernel_sync_send` stamps a per-cid seq and sends a datagram
//!   when the client has a live UDP path, else falls back to a WS frame —
//!   phones stay `wss`, native peers ride UDP, same world.
//! - Any datagram refreshes the path's keepalive; a silent path unbinds after
//!   [`UDP_TIMEOUT_US`] and sends fall back to WS (the client keeps a steady
//!   keepalive cadence — which doubles as the phone radio wake).
//! - Datagrams are capped at [`UDP_MAX_DATAGRAM`] bytes (overlay-shaved MTUs
//!   fragment silently above ~1200): oversized sync sends are dropped with a
//!   once-per-kernel warning, never a halt.
//!
//! Wire: WebSocket text frames (`<msg_id>:<payload>` convention is the loft
//! side's concern — the kernel passes payloads through verbatim).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::hash::{BuildHasher, Hash, Hasher};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use crate::database::Stores;
use crate::keys::{DbRef, Str};

/// One pumped event: connect (0), message (1), disconnect (2).
struct Event {
    cid: i64,
    kind: i64,
    payload: String,
}

/// A silent UDP path unbinds after this long; the client's keepalive cadence
/// (~500 ms — also the phone radio wake) keeps a live path far inside it.
const UDP_TIMEOUT_US: i64 = 3_000_000;
/// Datagram payload cap — overlay/VPN-shaved MTUs fragment silently above
/// ~1200 bytes, and a fragmented "fast path" is slower than the WS one.
const UDP_MAX_DATAGRAM: usize = 1200;

/// A client's bound UDP path (the source addr proven by the cookie handshake).
struct UdpPath {
    addr: SocketAddr,
    last_seen_us: i64,
    out_seq: i64,
}

/// One inbound conflation slot: the newest `S:` datagram from this client.
/// Latest-value semantics — a higher seq overwrites, a stale seq is dropped.
struct SyncSlot {
    seq: i64,
    payload: String,
    dirty: bool,
}

/// Per-connection network state beyond the TCP stream itself; lives and dies
/// with the connection slot (a reused cid starts fresh).
struct ClientNet {
    /// The UDP handshake cookie for this WS session ("" = no UDP listener).
    cookie: String,
    path: Option<UdpPath>,
    slot: SyncSlot,
}

impl ClientNet {
    fn new(cookie: String) -> Self {
        ClientNet {
            cookie,
            path: None,
            slot: SyncSlot {
                seq: -1,
                payload: String::new(),
                dirty: false,
            },
        }
    }
}

struct Kernel {
    listener: TcpListener,
    /// The state-sync datagram socket (same port number); `None` = bind
    /// failed at listen time (warned once; everything rides WS).
    udp: Option<UdpSocket>,
    /// Slot-indexed connections; `None` = free slot (cid = index, reused).
    conns: Vec<Option<TcpStream>>,
    /// Parallel to `conns`: cookie / UDP path / conflation slot per client.
    net: Vec<ClientNet>,
    events: VecDeque<Event>,
    /// The event handed out by the last `n_kernel_next_event`.
    last: Event,
    /// The slot handed out by the last `n_kernel_sync_next`.
    last_sync: (i64, i64, String),
    start: Instant,
    tick_interval_us: i64,
    last_tick_us: i64,
    warned_oversize: bool,
}

impl Kernel {
    fn now_us(&self) -> i64 {
        self.start.elapsed().as_micros() as i64
    }

    /// A fresh handshake cookie: 16 hex chars from the OS-seeded `RandomState`
    /// hasher mixed with the clock and cid.  Spoof-resistant on a LAN; not
    /// cryptographic (DTLS is the eventual answer for hostile networks).
    fn fresh_cookie(&self, cid: usize) -> String {
        if self.udp.is_none() {
            return String::new();
        }
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        self.start.elapsed().as_nanos().hash(&mut h);
        cid.hash(&mut h);
        format!("{:016x}", h.finish())
    }
}

thread_local! {
    static KERNEL: RefCell<Option<Kernel>> = const { RefCell::new(None) };
}

fn with_kernel<R>(f: impl FnOnce(&mut Kernel) -> R) -> Option<R> {
    KERNEL.with(|k| k.borrow_mut().as_mut().map(f))
}

// ── WS mechanics (unbuffered TcpStream reads — a BufReader would steal bytes
//    between pump turns; the frame reader mirrors the phase-00-patched pump:
//    peek the header non-blocking, then read the in-flight frame with a short
//    blocking timeout bound) ─────────────────────────────────────────────────

/// Upgrade a freshly-accepted stream: parse the HTTP request head, answer the
/// WebSocket handshake.  Returns the stream ready for frame traffic, or `None`
/// (not an upgrade / malformed — the connection is dropped).
fn ws_upgrade(mut stream: TcpStream) -> Option<TcpStream> {
    // The upgrade request is in flight from a live client; bound the read.
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    let mut buf = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    // Read header bytes until CRLFCRLF (unbuffered — no over-read).
    while !buf.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => buf.push(byte[0]),
            _ => return None,
        }
        if buf.len() > 16 * 1024 {
            return None; // header flood
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let mut key = None;
    let mut is_upgrade = false;
    for line in head.lines() {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "upgrade" => is_upgrade = v.trim().eq_ignore_ascii_case("websocket"),
                "sec-websocket-key" => key = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    let key = key?;
    if !is_upgrade {
        return None;
    }
    let accept = crate::serve::ws_accept_key(&key);
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream.write_all(resp.as_bytes()).ok()?;
    // Frame phase: short timeout bounds a torn frame; the peek keeps idle free.
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .ok()?;
    Some(stream)
}

enum FrameRead {
    None,
    Text(String),
    Closed,
}

/// Read one client frame if pending — **peek first** (idle costs µs, the
/// phase-00 lesson), then unbuffered `read_exact` for the in-flight frame.
fn read_frame(stream: &mut TcpStream) -> FrameRead {
    let mut hdr = [0u8; 2];
    let _ = stream.set_nonblocking(true);
    let peeked = stream.peek(&mut hdr);
    let _ = stream.set_nonblocking(false);
    match peeked {
        Ok(0) => return FrameRead::Closed,
        Ok(n) if n < 2 => return FrameRead::None, // header still arriving
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
            return FrameRead::None;
        }
        Err(_) => return FrameRead::Closed,
    }
    if stream.read_exact(&mut hdr).is_err() {
        return FrameRead::Closed;
    }
    let opcode = hdr[0] & 0x0F;
    let masked = hdr[1] & 0x80 != 0;
    let mut len = u64::from(hdr[1] & 0x7F);
    if len == 126 {
        let mut b = [0u8; 2];
        if stream.read_exact(&mut b).is_err() {
            return FrameRead::Closed;
        }
        len = u64::from(u16::from_be_bytes(b));
    } else if len == 127 {
        let mut b = [0u8; 8];
        if stream.read_exact(&mut b).is_err() {
            return FrameRead::Closed;
        }
        len = u64::from_be_bytes(b);
    }
    if len > 16 * 1024 * 1024 {
        return FrameRead::Closed; // bound a hostile length
    }
    let mut mask = [0u8; 4];
    if masked && stream.read_exact(&mut mask).is_err() {
        return FrameRead::Closed;
    }
    let mut payload = vec![0u8; usize::try_from(len).unwrap_or(0)];
    if stream.read_exact(&mut payload).is_err() {
        return FrameRead::Closed;
    }
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    match opcode {
        0x1 => FrameRead::Text(String::from_utf8_lossy(&payload).into_owned()),
        0x8 => FrameRead::Closed,
        0x9 => {
            // PING → PONG, then report nothing pending this turn.
            let _ = write_frame(stream, 0xA, &payload);
            FrameRead::None
        }
        _ => FrameRead::None, // binary/pong/continuation: ignored in v1
    }
}

/// Write one unmasked server frame.
fn write_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut frame = Vec::with_capacity(payload.len() + 10);
    frame.push(0x80 | opcode);
    if payload.len() <= 125 {
        frame.push(payload.len() as u8);
    } else if payload.len() <= 0xFFFF {
        frame.push(126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(payload);
    stream.write_all(&frame)
}

/// Bind with `SO_REUSEADDR` so a restarted server rebinds through TIME_WAIT —
/// the arcade flow (restart the cabinet mid-evening) depends on it; Rust's std
/// `TcpListener::bind` does not set it.
#[cfg(unix)]
fn bind_reuseaddr(port: u16) -> Option<TcpListener> {
    use std::os::fd::FromRawFd;
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return None;
        }
        let one: libc::c_int = 1;
        let _ = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            std::ptr::addr_of!(one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        let addr = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: port.to_be(),
            sin_addr: libc::in_addr { s_addr: 0 }, // 0.0.0.0
            sin_zero: [0; 8],
        };
        if libc::bind(
            fd,
            std::ptr::addr_of!(addr).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ) != 0
            || libc::listen(fd, 128) != 0
        {
            libc::close(fd);
            return None;
        }
        Some(TcpListener::from_raw_fd(fd))
    }
}

#[cfg(not(unix))]
fn bind_reuseaddr(port: u16) -> Option<TcpListener> {
    TcpListener::bind(("0.0.0.0", port)).ok()
}

// ── The natives (registered in native.rs; declared in lib/engine_host) ──────

/// `kernel_listen(port, tick_interval_us) -> boolean` — binds the WS listener
/// and, on the same port number, the state-sync UDP socket (05a).  A UDP bind
/// failure is a warning, not an error: everything rides WS until a restart.
pub fn n_kernel_listen(stores: &mut Stores, stack: &mut DbRef) {
    let tick_us = *stores.get::<i64>(stack);
    let port = *stores.get::<i64>(stack);
    let ok = bind_reuseaddr(port as u16)
        .map(|listener| {
            let _ = listener.set_nonblocking(true);
            let udp = match UdpSocket::bind(("0.0.0.0", port as u16)) {
                Ok(s) => {
                    let _ = s.set_nonblocking(true);
                    Some(s)
                }
                Err(e) => {
                    eprintln!("engine_host: UDP bind on {port} failed ({e}); state-sync rides WS");
                    None
                }
            };
            KERNEL.with(|k| {
                *k.borrow_mut() = Some(Kernel {
                    listener,
                    udp,
                    conns: Vec::new(),
                    net: Vec::new(),
                    events: VecDeque::new(),
                    last: Event {
                        cid: -1,
                        kind: -1,
                        payload: String::new(),
                    },
                    last_sync: (-1, -1, String::new()),
                    start: Instant::now(),
                    tick_interval_us: tick_us.max(1),
                    last_tick_us: 0,
                    warned_oversize: false,
                });
            });
        })
        .is_some();
    stores.put(stack, ok);
}

/// Drain every pending datagram into the conflation slots; bind/refresh UDP
/// paths; expire silent ones.  Called from the pump sweep — datagram arrivals
/// do NOT count as loop work (they conflate; the tick reads the newest).
fn pump_udp(k: &mut Kernel) {
    let Some(udp) = k.udp.as_ref() else {
        return;
    };
    let now = k.now_us();
    let mut buf = [0u8; 2048];
    loop {
        let (len, from) = match udp.recv_from(&mut buf) {
            Ok(r) => r,
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(_) => break,
        };
        let Ok(text) = std::str::from_utf8(&buf[..len]) else {
            continue; // not ours (binary lands with the wire-schema table)
        };
        if let Some(cookie) = text.strip_prefix("H:") {
            // Handshake: bind the source addr to the cid owning this cookie.
            let found = k
                .net
                .iter()
                .position(|n| !n.cookie.is_empty() && n.cookie == cookie);
            if let Some(cid) = found
                && k.conns.get(cid).is_some_and(Option::is_some)
            {
                k.net[cid].path = Some(UdpPath {
                    addr: from,
                    last_seen_us: now,
                    out_seq: 0,
                });
                let _ = udp.send_to(format!("A:{cid}").as_bytes(), from);
            }
            // Unknown cookie: silently dropped (spoof/staleness — the client
            // retries its hello until acked).
            continue;
        }
        // Everything else requires a bound path (identity = source addr).
        let Some(cid) = k
            .net
            .iter()
            .position(|n| n.path.as_ref().is_some_and(|p| p.addr == from))
        else {
            continue;
        };
        if let Some(p) = k.net[cid].path.as_mut() {
            p.last_seen_us = now; // any datagram is a keepalive
        }
        if let Some(rest) = text.strip_prefix("S:")
            && let Some((seq_s, payload)) = rest.split_once(':')
            && let Ok(seq) = seq_s.parse::<i64>()
        {
            let slot = &mut k.net[cid].slot;
            if seq > slot.seq {
                slot.seq = seq;
                slot.payload.clear();
                slot.payload.push_str(payload);
                slot.dirty = true;
            }
            // else: stale/reordered — discarded, never applied.
        }
        // "K:" (bare keepalive) needs nothing beyond the refresh above.
    }
    // Expire silent paths — sends fall back to WS transparently.
    for n in &mut k.net {
        if n.path
            .as_ref()
            .is_some_and(|p| now - p.last_seen_us > UDP_TIMEOUT_US)
        {
            n.path = None;
        }
    }
}

/// `kernel_pump() -> integer` — accept pending connections and drain every
/// ready frame into the event queue; returns the number of events enqueued.
/// One sweep, non-blocking throughout (idle clients cost a peek-µs each).
pub fn n_kernel_pump(stores: &mut Stores, stack: &mut DbRef) {
    let n = with_kernel(|k| {
        let mut added = 0i64;
        // Accept every pending connection this turn.
        loop {
            match k.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(s) = ws_upgrade(stream) {
                        let cid = k
                            .conns
                            .iter()
                            .position(Option::is_none)
                            .unwrap_or(k.conns.len());
                        let net = ClientNet::new(k.fresh_cookie(cid));
                        if cid == k.conns.len() {
                            k.conns.push(Some(s));
                            k.net.push(net);
                        } else {
                            k.conns[cid] = Some(s);
                            k.net[cid] = net; // reused cid starts fresh
                        }
                        k.events.push_back(Event {
                            cid: cid as i64,
                            kind: 0,
                            payload: String::new(),
                        });
                        added += 1;
                    }
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        // Drain ready frames from every connection (drain-to-empty per conn is
        // safe here: the queue grows in memory, and the LOFT loop budget-drains
        // the queue — the kernel never blocks on a quiet socket).
        for cid in 0..k.conns.len() {
            while let Some(stream) = k.conns[cid].as_mut() {
                match read_frame(stream) {
                    FrameRead::None => break,
                    FrameRead::Text(payload) => {
                        k.events.push_back(Event {
                            cid: cid as i64,
                            kind: 1,
                            payload,
                        });
                        added += 1;
                    }
                    FrameRead::Closed => {
                        disconnect(k, cid);
                        added += 1;
                        break;
                    }
                }
            }
        }
        // 05a: conflate pending state-sync datagrams (not counted as work —
        // the tick reads the newest; see pump_udp).
        pump_udp(k);
        added
    })
    .unwrap_or(0);
    stores.put(stack, n);
}

/// Close a connection slot: the cookie dies with the WS session (the UDP path
/// and conflation slot go with it) and a disconnect event is queued.
fn disconnect(k: &mut Kernel, cid: usize) {
    k.conns[cid] = None;
    k.net[cid] = ClientNet::new(String::new());
    k.events.push_back(Event {
        cid: cid as i64,
        kind: 2,
        payload: String::new(),
    });
}

/// `kernel_next_event() -> boolean` — pop the queue head into the event
/// getters below.
pub fn n_kernel_next_event(stores: &mut Stores, stack: &mut DbRef) {
    let got = with_kernel(|k| match k.events.pop_front() {
        Some(ev) => {
            k.last = ev;
            true
        }
        None => false,
    })
    .unwrap_or(false);
    stores.put(stack, got);
}

pub fn n_kernel_event_cid(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last.cid).unwrap_or(-1);
    stores.put(stack, v);
}

pub fn n_kernel_event_kind(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last.kind).unwrap_or(-1);
    stores.put(stack, v);
}

/// Destination-passing text return (@PLN10's convention for text-producing
/// natives): the caller allocates the destination and passes its `DbRef`;
/// routed by `is_text_dest_native("n_kernel_event_payload")`.
pub fn n_kernel_event_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = with_kernel(|k| k.last.payload.clone()).unwrap_or_default();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

/// `kernel_tick_due() -> boolean` — drift-free: when a tick is due, advance
/// `last_tick += interval` (never `= now`), so missed time is caught up tick
/// by tick instead of silently dropped.
pub fn n_kernel_tick_due(stores: &mut Stores, stack: &mut DbRef) {
    let due = with_kernel(|k| {
        let now = k.start.elapsed().as_micros() as i64;
        if now - k.last_tick_us >= k.tick_interval_us {
            if k.last_tick_us == 0 {
                k.last_tick_us = now; // first tick anchors the grid
            } else {
                k.last_tick_us += k.tick_interval_us;
            }
            true
        } else {
            false
        }
    })
    .unwrap_or(false);
    stores.put(stack, due);
}

/// `kernel_send(cid, msg) -> boolean`
pub fn n_kernel_send(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let cid = *stores.get::<i64>(stack);
    let ok = with_kernel(|k| {
        let Some(Some(stream)) = k.conns.get_mut(cid as usize) else {
            return false;
        };
        if write_frame(stream, 0x1, msg.as_bytes()).is_err() {
            disconnect(k, cid as usize);
            return false;
        }
        true
    })
    .unwrap_or(false);
    stores.put(stack, ok);
}

/// `kernel_broadcast(msg) -> integer` — send to every live connection;
/// returns the delivery count.  A failed send disconnects that client.
pub fn n_kernel_broadcast(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let n = with_kernel(|k| {
        let mut sent = 0i64;
        for cid in 0..k.conns.len() {
            let Some(stream) = k.conns[cid].as_mut() else {
                continue;
            };
            if write_frame(stream, 0x1, msg.as_bytes()).is_ok() {
                sent += 1;
            } else {
                disconnect(k, cid);
            }
        }
        sent
    })
    .unwrap_or(0);
    stores.put(stack, n);
}

/// `kernel_idle(max_us)` — sleep, but never past the next tick boundary.
/// Called by the loft loop only when a turn produced no work.
pub fn n_kernel_idle(stores: &mut Stores, stack: &mut DbRef) {
    let max_us = *stores.get::<i64>(stack);
    let sleep_us = with_kernel(|k| {
        let now = k.start.elapsed().as_micros() as i64;
        let until_tick = if k.last_tick_us == 0 {
            k.tick_interval_us
        } else {
            (k.last_tick_us + k.tick_interval_us - now).max(0)
        };
        max_us.clamp(0, until_tick.max(1))
    })
    .unwrap_or(max_us.max(0));
    std::thread::sleep(Duration::from_micros(sleep_us as u64));
}

/// `kernel_clients() -> integer` — live connection count (diagnostics).
pub fn n_kernel_clients(stores: &mut Stores, stack: &mut DbRef) {
    let n = with_kernel(|k| k.conns.iter().filter(|c| c.is_some()).count() as i64).unwrap_or(0);
    stores.put(stack, n);
}

// ── 05a — the state-sync UDP channel ────────────────────────────────────────

/// `udp_cookie(cid) -> text` — the client's UDP handshake cookie (destination-
/// passing).  Empty when there is no UDP listener or no such connection.  The
/// loft side transports it to the client inside its own protocol; the client
/// echoes it in a `H:<cookie>` datagram to bind its UDP path.
pub fn n_kernel_udp_cookie_dest(stores: &mut Stores, stack: &mut DbRef) {
    // The dest ref is pushed LAST by the dest-pass emitter, so it pops FIRST
    // (then the declared args in reverse) — the n_ymd_days_ago_dest order.
    let dest = *stores.get::<DbRef>(stack);
    let cid = *stores.get::<i64>(stack);
    let v = with_kernel(|k| {
        if k.conns.get(cid as usize).is_some_and(Option::is_some) {
            k.net[cid as usize].cookie.clone()
        } else {
            String::new()
        }
    })
    .unwrap_or_default();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

/// `udp_bound(cid) -> boolean` — does this client have a live UDP path?
pub fn n_kernel_udp_bound(stores: &mut Stores, stack: &mut DbRef) {
    let cid = *stores.get::<i64>(stack);
    let v =
        with_kernel(|k| k.net.get(cid as usize).is_some_and(|n| n.path.is_some())).unwrap_or(false);
    stores.put(stack, v);
}

/// `sync_send(cid, msg) -> boolean` — send a latest-value state frame: a
/// seq-stamped datagram over the client's UDP path when bound, else a plain
/// WS frame (phones stay `wss`, native peers ride UDP — same call, same
/// world).  Oversized datagrams are dropped with a once-per-kernel warning
/// (fragmentation is silent and slower than the WS path it would replace).
pub fn n_kernel_sync_send(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let cid = *stores.get::<i64>(stack);
    let ok = with_kernel(|k| {
        let Some(net) = k.net.get_mut(cid as usize) else {
            return false;
        };
        if let (Some(path), Some(udp)) = (net.path.as_mut(), k.udp.as_ref()) {
            path.out_seq += 1;
            let dgram = format!("S:{}:{msg}", path.out_seq);
            if dgram.len() > UDP_MAX_DATAGRAM {
                if !k.warned_oversize {
                    k.warned_oversize = true;
                    eprintln!(
                        "engine_host: sync_send dropped a {} B datagram (cap {} B; \
                         state frames must stay small — bulk belongs on the WS channel)",
                        dgram.len(),
                        UDP_MAX_DATAGRAM
                    );
                }
                return false;
            }
            return udp.send_to(dgram.as_bytes(), path.addr).is_ok();
        }
        // No UDP path: fall back to the reliable channel.
        let Some(Some(stream)) = k.conns.get_mut(cid as usize) else {
            return false;
        };
        if write_frame(stream, 0x1, msg.as_bytes()).is_err() {
            disconnect(k, cid as usize);
            return false;
        }
        true
    })
    .unwrap_or(false);
    stores.put(stack, ok);
}

/// `sync_next() -> boolean` — load the next dirty conflation slot (newest
/// state from one client) into the getters below and mark it read.  Draining
/// inside `on_tick` gives late-latch by construction: the freshest datagram
/// that arrived before the tick is the one read.
pub fn n_kernel_sync_next(stores: &mut Stores, stack: &mut DbRef) {
    let got = with_kernel(|k| {
        for cid in 0..k.net.len() {
            if k.net[cid].slot.dirty {
                k.net[cid].slot.dirty = false;
                k.last_sync = (
                    cid as i64,
                    k.net[cid].slot.seq,
                    k.net[cid].slot.payload.clone(),
                );
                return true;
            }
        }
        false
    })
    .unwrap_or(false);
    stores.put(stack, got);
}

pub fn n_kernel_sync_cid(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last_sync.0).unwrap_or(-1);
    stores.put(stack, v);
}

pub fn n_kernel_sync_seq(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last_sync.1).unwrap_or(-1);
    stores.put(stack, v);
}

/// Destination-passing text return — see `n_kernel_event_payload_dest`.
pub fn n_kernel_sync_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = with_kernel(|k| k.last_sync.2.clone()).unwrap_or_default();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}
