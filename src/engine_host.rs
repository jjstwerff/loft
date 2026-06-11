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
#[cfg(not(target_arch = "wasm32"))]
use std::hash::{BuildHasher, Hash, Hasher};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{ErrorKind, Read, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

use crate::database::Stores;
use crate::keys::{DbRef, Str};

/// One pumped event: connect (0), message (1), disconnect (2).
struct Event {
    cid: i64,
    kind: i64,
    payload: String,
}

#[cfg(not(target_arch = "wasm32"))]
/// A silent UDP path unbinds after this long; the client's keepalive cadence
/// (~500 ms — also the phone radio wake) keeps a live path far inside it.
const UDP_TIMEOUT_US: i64 = 3_000_000;
#[cfg(not(target_arch = "wasm32"))]
/// Datagram payload cap — overlay/VPN-shaved MTUs fragment silently above
/// ~1200 bytes, and a fragmented "fast path" is slower than the WS one.
const UDP_MAX_DATAGRAM: usize = 1200;

#[cfg(not(target_arch = "wasm32"))]
/// A client's bound UDP path (the source addr proven by the cookie handshake).
struct UdpPath {
    addr: SocketAddr,
    last_seen_us: i64,
}

/// One inbound conflation slot: the newest `S:` datagram from this peer for
/// one `(msg_id, key)`.  Latest-value semantics — a higher seq overwrites, a
/// stale seq is dropped.
struct SyncSlot {
    /// The `msg_id` parsed from the datagram's payload (`-1` = unframed).
    msg_id: i64,
    /// For a `sync_class_keyed` kind: the payload's first field (the entity
    /// id — "2:7,…" → "7"), so N entities keep N latest-values.  "" for
    /// unkeyed kinds (one latest-value per kind).
    key: String,
    seq: i64,
    payload: String,
    dirty: bool,
}

#[cfg(not(target_arch = "wasm32"))]
/// Per-connection network state beyond the TCP stream itself; lives and dies
/// with the connection slot (a reused cid starts fresh).
struct ClientNet {
    /// The UDP handshake cookie for this WS session ("" = no UDP listener).
    cookie: String,
    path: Option<UdpPath>,
    /// One outbound seq space per peer for ALL sync-channel sends — datagrams
    /// AND keyframes — so the receiver's slots totally order them even when
    /// the carriers race (a keyframe on TCP vs in-flight datagrams).
    out_seq: i64,
    /// Conflation slots, one per `msg_id` this client has sent (newest per
    /// sender PER MESSAGE KIND — two sync kinds never collapse each other).
    slots: Vec<SyncSlot>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ClientNet {
    fn new(cookie: String) -> Self {
        ClientNet {
            cookie,
            path: None,
            out_seq: 0,
            slots: Vec::new(),
        }
    }
}

// The wire-schema-as-data table, minimal form (user-directed 2026-06-10):
// `msg_id -> keyed?` for the kinds whose messages are **latest-value state**
// (the sync class — conflate, ride UDP when the peer can).  `keyed = true`
// (`sync_class_keyed`) conflates per the payload's first field — the entity
// id — so one kind carries N entities' latest-values ("state-sync keyed by
// cid" from the class table); `false` keeps one latest-value per kind.
// Everything undeclared is the event class (must-deliver, rides WS).
// Lives outside `Kernel` so declarations may precede `run()`.
thread_local! {
    static SYNC_IDS: RefCell<std::collections::HashMap<i64, bool>> =
        RefCell::new(std::collections::HashMap::new());
}

/// The leading `<digits>:` of a wire message, or `-1` when unframed.
fn msg_id_of(msg: &str) -> i64 {
    msg.split_once(':')
        .and_then(|(id, _)| id.parse::<i64>().ok())
        .unwrap_or(-1)
}

fn is_sync_msg(msg: &str) -> bool {
    let id = msg_id_of(msg);
    id >= 0 && SYNC_IDS.with(|s| s.borrow().contains_key(&id))
}

/// The conflation key for a payload: the first comma-field after the kind
/// for keyed kinds ("2:7,x,y" → "7"), "" for unkeyed/unframed ones.
fn sync_key_of(payload: &str) -> String {
    let id = msg_id_of(payload);
    let keyed = id >= 0 && SYNC_IDS.with(|s| s.borrow().get(&id).copied().unwrap_or(false));
    if !keyed {
        return String::new();
    }
    let body = payload.split_once(':').map_or("", |(_, b)| b);
    body.split(',').next().unwrap_or("").to_string()
}

/// Conflate one inbound `S:` payload into a slot set: per `msg_id`, a higher
/// seq overwrites, a stale/reordered seq is discarded.  Shared by both roles
/// (the listener's per-client slots and the connector's server slots) — the
/// queue machinery must never fork between them.
fn find_or_create_slot<'a>(
    slots: &'a mut Vec<SyncSlot>,
    payload: &str,
) -> Option<&'a mut SyncSlot> {
    // Bounded slot count — a peer inventing endless ids/keys cannot grow
    // memory; sync is loss-tolerant by definition, so over-cap drops.
    const SYNC_SLOTS_MAX: usize = 256;
    let msg_id = msg_id_of(payload);
    let key = sync_key_of(payload);
    match slots
        .iter_mut()
        .position(|s| s.msg_id == msg_id && s.key == key)
    {
        Some(i) => Some(&mut slots[i]),
        None if slots.len() >= SYNC_SLOTS_MAX => None,
        None => {
            slots.push(SyncSlot {
                msg_id,
                key,
                seq: -1,
                payload: String::new(),
                dirty: false,
            });
            slots.last_mut()
        }
    }
}

/// Ordered-carrier conflation: a sync-class message arriving over WS (a
/// phone's pose; a native client's pre-bind fallback).  The carrier is
/// ordered and lossless, so the arrival order IS the seq — continue the
/// slot's space, so a later datagram (the sender counts its sync sends
/// across BOTH carriers) still reads as newer.
fn conflate_ws(slots: &mut Vec<SyncSlot>, payload: &str) {
    if let Some(slot) = find_or_create_slot(slots, payload) {
        slot.seq += 1;
        slot.payload.clear();
        slot.payload.push_str(payload);
        slot.dirty = true;
    }
}

fn conflate_slot(slots: &mut Vec<SyncSlot>, seq: i64, payload: &str) {
    let Some(slot) = find_or_create_slot(slots, payload) else {
        return;
    };
    if seq > slot.seq {
        slot.seq = seq;
        slot.payload.clear();
        slot.payload.push_str(payload);
        slot.dirty = true;
    }
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static KERNEL: RefCell<Option<Kernel>> = const { RefCell::new(None) };
}

#[cfg(not(target_arch = "wasm32"))]
fn with_kernel<R>(f: impl FnOnce(&mut Kernel) -> R) -> Option<R> {
    KERNEL.with(|k| k.borrow_mut().as_mut().map(f))
}

// ── WS mechanics (unbuffered TcpStream reads — a BufReader would steal bytes
//    between pump turns; the frame reader mirrors the phase-00-patched pump:
//    peek the header non-blocking, then read the in-flight frame with a short
//    blocking timeout bound) ─────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
/// Upgrade a freshly-accepted stream: parse the HTTP request head, answer the
/// WebSocket handshake.  Returns the stream ready for frame traffic, or `None`
/// (not an upgrade / malformed — the connection is dropped).
///
/// A non-empty `udp_cookie` rides the 101 response as an `X-Loft-UDP` header —
/// the transport negotiation is **fully kernel-internal** (user directive,
/// 2026-06-10: the game developer is never bothered with transport).  Browsers
/// cannot read upgrade-response headers from JS and ignore it; a native
/// client's kernel reads it and auto-hellos, earning the UDP fast path with
/// zero meaning-level code on either side.
fn ws_upgrade(mut stream: TcpStream, udp_cookie: &str) -> Option<TcpStream> {
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
    let udp_header = if udp_cookie.is_empty() {
        String::new()
    } else {
        format!("X-Loft-UDP: {udp_cookie}\r\n")
    };
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n{udp_header}\r\n"
    );
    stream.write_all(resp.as_bytes()).ok()?;
    // Frame phase: short timeout bounds a torn frame; the peek keeps idle free.
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .ok()?;
    Some(stream)
}

#[cfg(not(target_arch = "wasm32"))]
enum FrameRead {
    None,
    Text(String),
    Closed,
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(all(not(unix), not(target_arch = "wasm32")))]
fn bind_reuseaddr(port: u16) -> Option<TcpListener> {
    TcpListener::bind(("0.0.0.0", port)).ok()
}

// ── The natives (registered in native.rs; declared in lib/engine_host) ──────

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_listen(port, tick_interval_us) -> boolean` — binds the WS listener
/// and, on the same port number, the state-sync UDP socket (05a).  A UDP bind
/// failure is a warning, not an error: everything rides WS until a restart.
pub fn n_kernel_listen(stores: &mut Stores, stack: &mut DbRef) {
    let tick_us = *stores.get::<i64>(stack);
    let port = *stores.get::<i64>(stack);
    let ok = listen_impl(port, tick_us);
    stores.put(stack, ok);
}

/// The listen body, shared by the bytecode-stack native and the typed
/// (`--native` codegen) twin — one implementation, two calling conventions.
#[cfg(not(target_arch = "wasm32"))]
fn listen_impl(port: i64, tick_us: i64) -> bool {
    bind_reuseaddr(port as u16)
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
        .is_some()
}

#[cfg(not(target_arch = "wasm32"))]
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
            // Conflate per (sender, msg_id): two sync kinds from one client
            // each keep their own newest; stale/reordered seqs drop.
            conflate_slot(&mut k.net[cid].slots, seq, payload);
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
#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_pump(stores: &mut Stores, stack: &mut DbRef) {
    let n = with_kernel(pump_kernel).unwrap_or(0);
    stores.put(stack, n);
}

/// The pump body, shared by both calling conventions.
#[cfg(not(target_arch = "wasm32"))]
fn pump_kernel(k: &mut Kernel) -> i64 {
    {
        let mut added = 0i64;
        // Accept every pending connection this turn.
        loop {
            match k.listener.accept() {
                Ok((stream, _)) => {
                    let cid = k
                        .conns
                        .iter()
                        .position(Option::is_none)
                        .unwrap_or(k.conns.len());
                    // Mint the cookie BEFORE the upgrade so the 101 response
                    // carries the same value `ClientNet` stores — the whole
                    // negotiation stays kernel-internal.
                    let cookie = k.fresh_cookie(cid);
                    if let Some(s) = ws_upgrade(stream, &cookie) {
                        let net = ClientNet::new(cookie);
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
                        // Wire-schema routing on the inbound reliable carrier
                        // too: a sync-class frame (a phone's pose) conflates
                        // exactly like a datagram — the server script reads
                        // ONE surface (sync_next) regardless of the sender's
                        // transport.
                        if is_sync_msg(&payload) {
                            conflate_ws(&mut k.net[cid].slots, &payload);
                        } else {
                            k.events.push_back(Event {
                                cid: cid as i64,
                                kind: 1,
                                payload,
                            });
                            added += 1;
                        }
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
    }
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_event_cid(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last.cid).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_event_kind(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last.kind).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_tick_due() -> boolean` — drift-free: when a tick is due, advance
/// `last_tick += interval` (never `= now`), so missed time is caught up tick
/// by tick instead of silently dropped.
pub fn n_kernel_tick_due(stores: &mut Stores, stack: &mut DbRef) {
    let due = with_kernel(tick_due_kernel).unwrap_or(false);
    stores.put(stack, due);
}

#[cfg(not(target_arch = "wasm32"))]
fn tick_due_kernel(k: &mut Kernel) -> bool {
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
}

#[cfg(not(target_arch = "wasm32"))]
/// Deliver one message to one client, routed by the wire-schema table: a
/// sync-class message (`sync_class(msg_id)`) rides a seq-stamped datagram
/// when the client's UDP path is bound; everything else — and every client
/// without a path — gets a WS frame.  ONE function, the fastest path that
/// client supports; meaning never branches on transport.
fn deliver(k: &mut Kernel, cid: usize, msg: &str, sync: bool) -> bool {
    if sync
        && let Some(net) = k.net.get_mut(cid)
        && let Some(addr) = net.path.as_ref().map(|p| p.addr)
        && let Some(udp) = k.udp.as_ref()
    {
        net.out_seq += 1;
        let dgram = format!("S:{}:{msg}", net.out_seq);
        if dgram.len() > UDP_MAX_DATAGRAM {
            if !k.warned_oversize {
                k.warned_oversize = true;
                eprintln!(
                    "engine_host: dropped a {} B sync datagram (cap {} B; \
                     state frames must stay small — bulk belongs on the WS channel)",
                    dgram.len(),
                    UDP_MAX_DATAGRAM
                );
            }
            return false;
        }
        return udp.send_to(dgram.as_bytes(), addr).is_ok();
    }
    // Event class, or no UDP path: the reliable channel.
    let Some(Some(stream)) = k.conns.get_mut(cid) else {
        return false;
    };
    if write_frame(stream, 0x1, msg.as_bytes()).is_err() {
        disconnect(k, cid);
        return false;
    }
    true
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_send(cid, msg) -> boolean` — class-routed delivery (see `deliver`).
pub fn n_kernel_send(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let cid = *stores.get::<i64>(stack);
    let sync = is_sync_msg(&msg);
    let ok = with_kernel(|k| deliver(k, cid as usize, &msg, sync)).unwrap_or(false);
    stores.put(stack, ok);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_broadcast(msg) -> integer` — class-routed delivery to every live
/// connection (each client gets its own fastest path); returns the delivery
/// count.  A failed WS send disconnects that client.
pub fn n_kernel_broadcast(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let sync = is_sync_msg(&msg);
    let n = with_kernel(|k| {
        let mut sent = 0i64;
        for cid in 0..k.conns.len() {
            if k.conns[cid].is_none() {
                continue;
            }
            if deliver(k, cid, &msg, sync) {
                sent += 1;
            }
        }
        sent
    })
    .unwrap_or(0);
    stores.put(stack, n);
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_clients() -> integer` — live connection count (diagnostics).
pub fn n_kernel_clients(stores: &mut Stores, stack: &mut DbRef) {
    let n = with_kernel(|k| k.conns.iter().filter(|c| c.is_some()).count() as i64).unwrap_or(0);
    stores.put(stack, n);
}

// ── 05a — the state-sync UDP channel ────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
/// `udp_bound(cid) -> boolean` — does this client have a live UDP path?
pub fn n_kernel_udp_bound(stores: &mut Stores, stack: &mut DbRef) {
    let cid = *stores.get::<i64>(stack);
    let v =
        with_kernel(|k| k.net.get(cid as usize).is_some_and(|n| n.path.is_some())).unwrap_or(false);
    stores.put(stack, v);
}

/// `sync_class(msg_id)` — declare a `msg_id` as latest-value state in the
/// wire-schema table: `send`/`broadcast` then route messages of that kind to
/// the sync channel (datagram when the client can, WS frame when it can't)
/// and inbound datagrams of that kind conflate.  Data, not a per-call API —
/// the developer states what a message IS once; the kernel picks transports.
pub fn n_kernel_sync_class(stores: &mut Stores, stack: &mut DbRef) {
    let msg_id = *stores.get::<i64>(stack);
    SYNC_IDS.with(|s| {
        s.borrow_mut().insert(msg_id, false);
    });
}

/// `sync_class_keyed(msg_id)` — a sync kind whose payload's FIRST field is an
/// entity id: conflation keeps the newest per (peer, kind, entity), so one
/// kind carries N entities' latest-values ("state-sync keyed by cid").
pub fn n_kernel_sync_class_keyed(stores: &mut Stores, stack: &mut DbRef) {
    let msg_id = *stores.get::<i64>(stack);
    SYNC_IDS.with(|s| {
        s.borrow_mut().insert(msg_id, true);
    });
}

#[cfg(not(target_arch = "wasm32"))]
/// Deliver one PRIORITY KEYFRAME — a sync-stream sample promoted to
/// must-deliver (the class table's discontinuity rule: a bounce must not be
/// lost the way a smooth sample may be).  Rides the reliable channel, stamped
/// in the SAME seq space as the datagrams so the receiver's slots totally
/// order the two carriers: a bound connector gets an `S:`-framed WS frame
/// (it lands in the conflation slots, and any in-flight OLDER datagram is
/// then discarded as stale); an unbound peer (a web page) gets the plain
/// message — which is already its normal delivery.  WHAT counts as a
/// discontinuity is meaning; only delivery is mechanics.
fn deliver_keyframe(k: &mut Kernel, cid: usize, msg: &str) -> bool {
    let Some(net) = k.net.get_mut(cid) else {
        return false;
    };
    let framed = if net.path.is_some() {
        net.out_seq += 1;
        Some(format!("S:{}:{msg}", net.out_seq))
    } else {
        None
    };
    let Some(Some(stream)) = k.conns.get_mut(cid) else {
        return false;
    };
    let wire = framed.as_deref().unwrap_or(msg);
    if write_frame(stream, 0x1, wire.as_bytes()).is_err() {
        disconnect(k, cid);
        return false;
    }
    true
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_keyframe(cid, msg) -> boolean` — see `deliver_keyframe`.
pub fn n_kernel_keyframe(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let cid = *stores.get::<i64>(stack);
    let ok = with_kernel(|k| deliver_keyframe(k, cid as usize, &msg)).unwrap_or(false);
    stores.put(stack, ok);
}

#[cfg(not(target_arch = "wasm32"))]
/// `sync_next() -> boolean` — load the next dirty conflation slot (newest
/// state from one client) into the getters below and mark it read.  Draining
/// inside `on_tick` gives late-latch by construction: the freshest datagram
/// that arrived before the tick is the one read.
pub fn n_kernel_sync_next(stores: &mut Stores, stack: &mut DbRef) {
    let got = with_kernel(sync_next_kernel).unwrap_or(false);
    stores.put(stack, got);
}

#[cfg(not(target_arch = "wasm32"))]
fn sync_next_kernel(k: &mut Kernel) -> bool {
    for cid in 0..k.net.len() {
        for slot in &mut k.net[cid].slots {
            if slot.dirty {
                slot.dirty = false;
                // The payload carries its own `<msg_id>:` framing, so the
                // drained message reads exactly like an event payload.
                k.last_sync = (cid as i64, slot.seq, slot.payload.clone());
                return true;
            }
        }
    }
    false
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_sync_cid(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last_sync.0).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_sync_seq(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_kernel(|k| k.last_sync.1).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// Destination-passing text return — see `n_kernel_event_payload_dest`.
pub fn n_kernel_sync_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = with_kernel(|k| k.last_sync.2.clone()).unwrap_or_default();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

// ── The connector role — the client-side kernel (one core, two roles) ───────
//
// A native client's half of the auto-path: connect + WS upgrade, read the
// `X-Loft-UDP` cookie from the 101 head, auto-hello until acked, keepalive
// cadence (the phone-radio wake), class-routed `client_send`, and inbound
// conflation through the SAME `conflate_slot`/`SYNC_IDS` machinery the
// listener uses — the queue semantics never fork between roles.  The loft
// surface is `run_client(host, port, tick_us, on_event, on_tick)`, which
// returns when the server connection dies (a connector without a server has
// nothing left to do — unlike `run`, which serves forever).

#[cfg(not(target_arch = "wasm32"))]
/// Hello retry cadence while unbound (the ack races real traffic, so this is
/// a retry, not a timeout).
const HELLO_INTERVAL_US: i64 = 200_000;
#[cfg(not(target_arch = "wasm32"))]
/// Keepalive cadence once bound — far inside the listener's 3 s path timeout.
const KEEPALIVE_INTERVAL_US: i64 = 500_000;

#[cfg(not(target_arch = "wasm32"))]
struct ClientKernel {
    conn: TcpStream,
    udp: Option<UdpSocket>,
    /// From the 101 head; "" = the server offered no UDP (everything WS).
    cookie: String,
    udp_bound: bool,
    last_hello_us: i64,
    last_keepalive_us: i64,
    out_seq: i64,
    /// Inbound conflation slots for the server's sync traffic (per msg_id).
    slots: Vec<SyncSlot>,
    /// (seq, payload) handed out by the last `client_sync_next`.
    last_sync: (i64, String),
    events: VecDeque<Event>,
    last: Event,
    start: Instant,
    tick_interval_us: i64,
    last_tick_us: i64,
    /// False once the server connection closed — `run_client` then returns.
    alive: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl ClientKernel {
    fn now_us(&self) -> i64 {
        self.start.elapsed().as_micros() as i64
    }
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static CLIENT: RefCell<Option<ClientKernel>> = const { RefCell::new(None) };
}

#[cfg(not(target_arch = "wasm32"))]
fn with_client<R>(f: impl FnOnce(&mut ClientKernel) -> R) -> Option<R> {
    CLIENT.with(|c| c.borrow_mut().as_mut().map(f))
}

#[cfg(not(target_arch = "wasm32"))]
/// Client→server frames are masked (RFC 6455 requires it; `read_frame` on the
/// listener side already handles both).  The mask key needs no randomness for
/// security — it exists to defeat broken transparent proxies — so a cheap
/// counter-derived key is fine.
fn write_frame_masked(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | opcode);
    if payload.len() <= 125 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= 0xFFFF {
        frame.push(0x80 | 0x7E);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 0x7F);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    let mask = (payload.len() as u32)
        .wrapping_mul(0x9E37_79B9)
        .to_be_bytes();
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    stream.write_all(&frame)
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_connect(host, port, tick_interval_us) -> boolean` — TCP connect,
/// WS upgrade (the fixed RFC sample key: the key field is anti-cache, not
/// auth), capture the `X-Loft-UDP` cookie, prepare the UDP socket.  Queues a
/// kind-0 event so `on_event` sees the connect like the listener side does.
pub fn n_kernel_connect(stores: &mut Stores, stack: &mut DbRef) {
    let tick_us = *stores.get::<i64>(stack);
    let port = *stores.get::<i64>(stack);
    let host = stores.get::<Str>(stack).str().to_owned();
    let ok = client_connect(&host, port as u16, tick_us).is_some();
    stores.put(stack, ok);
}

#[cfg(not(target_arch = "wasm32"))]
fn client_connect(host: &str, port: u16, tick_us: i64) -> Option<()> {
    use std::net::ToSocketAddrs;
    let addr = (host, port).to_socket_addrs().ok()?.next()?;
    let mut conn = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).ok()?;
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let req = format!(
        "GET /ws HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
    conn.write_all(req.as_bytes()).ok()?;
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match conn.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return None,
        }
        if head.len() > 16 * 1024 {
            return None;
        }
    }
    let head = String::from_utf8_lossy(&head);
    if !head.contains(" 101 ") {
        return None;
    }
    let cookie = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("x-loft-udp")
                .then(|| v.trim().to_string())
        })
        .unwrap_or_default();
    // Frame phase: the listener's peek pattern bounds idle reads.
    conn.set_read_timeout(Some(Duration::from_millis(20)))
        .ok()?;
    let udp = if cookie.is_empty() {
        None
    } else {
        UdpSocket::bind("0.0.0.0:0").ok().and_then(|s| {
            s.connect(addr).ok()?;
            s.set_nonblocking(true).ok()?;
            Some(s)
        })
    };
    CLIENT.with(|c| {
        *c.borrow_mut() = Some(ClientKernel {
            conn,
            udp,
            cookie,
            udp_bound: false,
            last_hello_us: i64::MIN / 2,
            last_keepalive_us: 0,
            out_seq: 0,
            slots: Vec::new(),
            last_sync: (-1, String::new()),
            events: {
                let mut q = VecDeque::new();
                q.push_back(Event {
                    cid: 0,
                    kind: 0,
                    payload: String::new(),
                });
                q
            },
            last: Event {
                cid: -1,
                kind: -1,
                payload: String::new(),
            },
            start: Instant::now(),
            tick_interval_us: tick_us.max(1),
            last_tick_us: 0,
            alive: true,
        });
        Some(())
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_client_pump() -> integer` — drain server WS frames into the event
/// queue and server datagrams into the conflation slots; drive the hello /
/// keepalive cadences.  Datagram arrivals are not counted as work (they
/// conflate; the tick reads the newest — same rule as the listener).
pub fn n_kernel_client_pump(stores: &mut Stores, stack: &mut DbRef) {
    let n = with_client(pump_client).unwrap_or(0);
    stores.put(stack, n);
}

/// The connector pump body, shared by both calling conventions.
#[cfg(not(target_arch = "wasm32"))]
fn pump_client(c: &mut ClientKernel) -> i64 {
    {
        let mut added = 0i64;
        // WS frames: events from the server — except `S:`-framed keyframes,
        // which are promoted sync samples riding the reliable channel in the
        // datagram seq space: they conflate (the tick reads the newest, and
        // an in-flight older datagram becomes stale), never queue.
        while c.alive {
            match read_frame(&mut c.conn) {
                FrameRead::None => break,
                FrameRead::Text(payload) => {
                    if let Some(rest) = payload.strip_prefix("S:")
                        && let Some((seq_s, body)) = rest.split_once(':')
                        && let Ok(seq) = seq_s.parse::<i64>()
                    {
                        conflate_slot(&mut c.slots, seq, body);
                        continue;
                    }
                    // Plain sync-class frames (the pre-bind fallback) conflate
                    // too — the script's surface is sync_next either way.
                    if is_sync_msg(&payload) {
                        conflate_ws(&mut c.slots, &payload);
                        continue;
                    }
                    c.events.push_back(Event {
                        cid: 0,
                        kind: 1,
                        payload,
                    });
                    added += 1;
                }
                FrameRead::Closed => {
                    c.alive = false;
                    c.udp_bound = false;
                    c.events.push_back(Event {
                        cid: 0,
                        kind: 2,
                        payload: String::new(),
                    });
                    added += 1;
                }
            }
        }
        // Datagrams: the ack and the server's sync traffic.
        if let Some(udp) = c.udp.as_ref() {
            // Measurement-only loss injection (@PLN18 phase 04, the loss%
            // axis): `LOFT_UDP_DROP_NTH=N` drops every Nth received sync
            // datagram, deterministically — loopback has no real loss, and
            // the sync class's claim is graceful degradation under it.
            // Read once; 0 = off.
            thread_local! {
                static DROP_NTH: u64 = std::env::var("LOFT_UDP_DROP_NTH")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                static RECV_COUNT: RefCell<u64> = const { RefCell::new(0) };
            }
            let drop_nth = DROP_NTH.with(|d| *d);
            let mut buf = [0u8; 2048];
            loop {
                let len = match udp.recv(&mut buf) {
                    Ok(l) => l,
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => break,
                };
                let Ok(text) = std::str::from_utf8(&buf[..len]) else {
                    continue;
                };
                if text.starts_with("A:") {
                    c.udp_bound = true;
                } else if let Some(rest) = text.strip_prefix("S:")
                    && let Some((seq_s, payload)) = rest.split_once(':')
                    && let Ok(seq) = seq_s.parse::<i64>()
                {
                    if drop_nth > 0 {
                        let nth = RECV_COUNT.with(|r| {
                            let mut r = r.borrow_mut();
                            *r += 1;
                            *r
                        });
                        if nth.is_multiple_of(drop_nth) {
                            continue; // injected loss — the datagram never existed
                        }
                    }
                    conflate_slot(&mut c.slots, seq, payload);
                }
            }
            // Hello until acked; keepalive once bound (the radio wake).
            let now = c.now_us();
            if !c.udp_bound && !c.cookie.is_empty() && now - c.last_hello_us > HELLO_INTERVAL_US {
                let _ = udp.send(format!("H:{}", c.cookie).as_bytes());
                c.last_hello_us = now;
            }
            if c.udp_bound && now - c.last_keepalive_us > KEEPALIVE_INTERVAL_US {
                let _ = udp.send(b"K:");
                c.last_keepalive_us = now;
            }
        }
        added
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_client_alive() -> boolean` — false once the server connection
/// closed; `run_client`'s loop condition.
pub fn n_kernel_client_alive(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_client(|c| c.alive).unwrap_or(false);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_client_next_event(stores: &mut Stores, stack: &mut DbRef) {
    let got = with_client(|c| match c.events.pop_front() {
        Some(ev) => {
            c.last = ev;
            true
        }
        None => false,
    })
    .unwrap_or(false);
    stores.put(stack, got);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_client_event_kind(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_client(|c| c.last.kind).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// Destination-passing text return — see `n_kernel_event_payload_dest`.
pub fn n_kernel_client_event_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = with_client(|c| c.last.payload.clone()).unwrap_or_default();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_client_tick_due() -> boolean` — the listener's drift-free rule.
pub fn n_kernel_client_tick_due(stores: &mut Stores, stack: &mut DbRef) {
    let due = with_client(tick_due_client).unwrap_or(false);
    stores.put(stack, due);
}

#[cfg(not(target_arch = "wasm32"))]
fn tick_due_client(c: &mut ClientKernel) -> bool {
    let now = c.now_us();
    if now - c.last_tick_us >= c.tick_interval_us {
        if c.last_tick_us == 0 {
            c.last_tick_us = now;
        } else {
            c.last_tick_us += c.tick_interval_us;
        }
        true
    } else {
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// `kernel_client_idle(max_us)` — sleep, capped at the next tick boundary.
pub fn n_kernel_client_idle(stores: &mut Stores, stack: &mut DbRef) {
    let max_us = *stores.get::<i64>(stack);
    let sleep_us = with_client(|c| {
        let now = c.now_us();
        let until_tick = if c.last_tick_us == 0 {
            c.tick_interval_us
        } else {
            (c.last_tick_us + c.tick_interval_us - now).max(0)
        };
        max_us.clamp(0, until_tick.max(1))
    })
    .unwrap_or(max_us.max(0));
    std::thread::sleep(Duration::from_micros(sleep_us as u64));
}

#[cfg(not(target_arch = "wasm32"))]
/// `client_send(msg) -> boolean` — class-routed, mirroring the listener's
/// `deliver`: a sync-class message rides a seq-stamped datagram once the
/// path is acked; everything else — and everything before the ack — goes as
/// a masked WS frame.
pub fn n_kernel_client_send(stores: &mut Stores, stack: &mut DbRef) {
    let msg = stores.get::<Str>(stack).str().to_owned();
    let sync = is_sync_msg(&msg);
    let ok = with_client(|c| client_send_impl(c, &msg, sync)).unwrap_or(false);
    stores.put(stack, ok);
}

/// The connector send body, shared by both calling conventions.
#[cfg(not(target_arch = "wasm32"))]
fn client_send_impl(c: &mut ClientKernel, msg: &str, sync: bool) -> bool {
    if !c.alive {
        return false;
    }
    if sync {
        // One counter across BOTH carriers: pre-bind WS sync sends count
        // too, so the first datagram after binding is newer than every
        // ordered-carrier send the server already conflated.
        c.out_seq += 1;
    }
    if sync
        && c.udp_bound
        && let Some(udp) = c.udp.as_ref()
    {
        let dgram = format!("S:{}:{msg}", c.out_seq);
        if dgram.len() > UDP_MAX_DATAGRAM {
            return false; // state frames must stay small (see the listener)
        }
        return udp.send(dgram.as_bytes()).is_ok();
    }
    if write_frame_masked(&mut c.conn, 0x1, msg.as_bytes()).is_err() {
        c.alive = false;
        c.udp_bound = false;
        c.events.push_back(Event {
            cid: 0,
            kind: 2,
            payload: String::new(),
        });
        return false;
    }
    true
}

#[cfg(not(target_arch = "wasm32"))]
/// `client_sync_next() -> boolean` — drain the newest unread server state,
/// one slot per call (drain inside `on_tick`: late-latch, the listener rule).
pub fn n_kernel_client_sync_next(stores: &mut Stores, stack: &mut DbRef) {
    let got = with_client(sync_next_client).unwrap_or(false);
    stores.put(stack, got);
}

#[cfg(not(target_arch = "wasm32"))]
fn sync_next_client(c: &mut ClientKernel) -> bool {
    for slot in &mut c.slots {
        if slot.dirty {
            slot.dirty = false;
            c.last_sync = (slot.seq, slot.payload.clone());
            return true;
        }
    }
    false
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_client_sync_seq(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_client(|c| c.last_sync.0).unwrap_or(-1);
    stores.put(stack, v);
}

#[cfg(not(target_arch = "wasm32"))]
/// Destination-passing text return — see `n_kernel_event_payload_dest`.
pub fn n_kernel_client_sync_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = with_client(|c| c.last_sync.1.clone()).unwrap_or_default();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

#[cfg(not(target_arch = "wasm32"))]
/// `client_udp_bound() -> boolean` — read-only introspection (diagnostics).
pub fn n_kernel_client_udp_bound(stores: &mut Stores, stack: &mut DbRef) {
    let v = with_client(|c| c.udp_bound).unwrap_or(false);
    stores.put(stack, v);
}

/// `kernel_client_frame()` — the per-turn yield point in `run_client`'s loop:
/// a no-op on native (the loop owns its thread), a frame-yield in the browser
/// (the tab must return to the event loop every turn).  Lives in the SHARED
/// lib source so scripts never see the difference.
pub fn n_kernel_client_frame(_stores: &mut Stores, _stack: &mut DbRef) {}

/// `default_host() -> text` — the host a client should connect to when the
/// script doesn't say: `LOFT_HOST` env, else loopback.  The browser variant
/// returns the page's serving origin (the cabinet).  Destination-passing.
pub fn n_kernel_default_host_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v = std::env::var("LOFT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

// ── The BROWSER kernel (@PLN18 phase 07) — the same loft script on a phone ──
//
// The connector role rebuilt on what the browser already provides: the
// `host_ws_*` bridge as the pump, the event loop as the idle (via the
// frame-yield contract), `time_ticks` as the tick grid.  The pure machinery
// above (conflation, the wire-schema table, the queues) is THE SAME CODE —
// one implementation, two targets, so the never-fork rule holds by
// construction.  Registered under the SAME `n_kernel_*` symbols
// (`native.rs::KERNEL_FUNCTIONS_WASM`), so `lib/engine_host`'s loft source
// — and every script over it — is shared verbatim.
#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
pub mod browser {
    use super::*;
    use crate::wasm::{
        host_origin_host, host_time_ticks, host_ws_connect, host_ws_last_message, host_ws_ready,
        host_ws_recv, host_ws_send,
    };

    struct BrowserClient {
        ws: i32,
        alive: bool,
        /// The async half of "connect blocks until upgraded": kind-0 fires
        /// and the outbox flushes only once the socket reports open.
        opened: bool,
        outbox: Vec<String>,
        events: VecDeque<Event>,
        last: Event,
        slots: Vec<SyncSlot>,
        last_sync: (i64, String),
        out_seq: i64,
        tick_interval_us: i64,
        last_tick_us: i64,
    }

    thread_local! {
        static CLIENT: RefCell<Option<BrowserClient>> = const { RefCell::new(None) };
    }

    fn with_client<R>(f: impl FnOnce(&mut BrowserClient) -> R) -> Option<R> {
        CLIENT.with(|c| c.borrow_mut().as_mut().map(f))
    }

    /// `kernel_connect(host, port, tick_interval_us) -> boolean` — open the
    /// browser WebSocket (async; the pump completes the handshake contract).
    pub fn n_kernel_connect(stores: &mut Stores, stack: &mut DbRef) {
        let tick_us = *stores.get::<i64>(stack);
        let port = *stores.get::<i64>(stack);
        let host = stores.get::<Str>(stack).str().to_owned();
        let ws = host_ws_connect(&format!("ws://{host}:{port}/ws"));
        let ok = ws >= 0;
        if ok {
            CLIENT.with(|c| {
                *c.borrow_mut() = Some(BrowserClient {
                    ws,
                    alive: true,
                    opened: false,
                    outbox: Vec::new(),
                    events: VecDeque::new(),
                    last: Event {
                        cid: -1,
                        kind: -1,
                        payload: String::new(),
                    },
                    slots: Vec::new(),
                    last_sync: (-1, String::new()),
                    out_seq: 0,
                    tick_interval_us: tick_us.max(1),
                    last_tick_us: 0,
                });
            });
        }
        stores.put(stack, ok);
    }

    /// `kernel_client_pump() -> integer` — drain browser-queued frames into
    /// the SAME machinery the native connector uses; fire kind-0 + flush the
    /// outbox once the socket opens.
    pub fn n_kernel_client_pump(stores: &mut Stores, stack: &mut DbRef) {
        let n = with_client(|c| {
            let mut added = 0i64;
            if !c.opened && host_ws_ready(c.ws) {
                c.opened = true;
                c.events.push_back(Event {
                    cid: 0,
                    kind: 0,
                    payload: String::new(),
                });
                added += 1;
                for msg in std::mem::take(&mut c.outbox) {
                    let _ = host_ws_send(c.ws, &msg, false);
                }
            }
            while c.alive && host_ws_recv(c.ws) == 1 {
                let payload = host_ws_last_message();
                // Keyframes ride `S:`-framed reliable frames for BOUND peers
                // only — a browser is never bound, but parse defensively.
                if let Some(rest) = payload.strip_prefix("S:")
                    && let Some((seq_s, body)) = rest.split_once(':')
                    && let Ok(seq) = seq_s.parse::<i64>()
                {
                    conflate_slot(&mut c.slots, seq, body);
                    continue;
                }
                if is_sync_msg(&payload) {
                    conflate_ws(&mut c.slots, &payload);
                    continue;
                }
                c.events.push_back(Event {
                    cid: 0,
                    kind: 1,
                    payload,
                });
                added += 1;
            }
            added
        })
        .unwrap_or(0);
        stores.put(stack, n);
    }

    pub fn n_kernel_client_alive(stores: &mut Stores, stack: &mut DbRef) {
        let v = with_client(|c| c.alive).unwrap_or(false);
        stores.put(stack, v);
    }

    pub fn n_kernel_client_next_event(stores: &mut Stores, stack: &mut DbRef) {
        let got = with_client(|c| match c.events.pop_front() {
            Some(ev) => {
                c.last = ev;
                true
            }
            None => false,
        })
        .unwrap_or(false);
        stores.put(stack, got);
    }

    pub fn n_kernel_client_event_kind(stores: &mut Stores, stack: &mut DbRef) {
        let v = with_client(|c| c.last.kind).unwrap_or(-1);
        stores.put(stack, v);
    }

    pub fn n_kernel_client_event_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
        let dest = *stores.get::<DbRef>(stack);
        let v = with_client(|c| c.last.payload.clone()).unwrap_or_default();
        stores
            .store_mut(&dest)
            .addr_mut::<String>(dest.rec, dest.pos)
            .push_str(&v);
    }

    /// Drift-free tick on the bridge clock (µs via `time_ticks`).
    pub fn n_kernel_client_tick_due(stores: &mut Stores, stack: &mut DbRef) {
        let due = with_client(|c| {
            let now = host_time_ticks();
            if now - c.last_tick_us >= c.tick_interval_us {
                if c.last_tick_us == 0 {
                    c.last_tick_us = now;
                } else {
                    c.last_tick_us += c.tick_interval_us;
                }
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
        stores.put(stack, due);
    }

    /// `kernel_client_idle(max_us)` — a no-op in the browser: the event loop
    /// IS the idle (the per-turn frame yield returns control to it).
    pub fn n_kernel_client_idle(stores: &mut Stores, stack: &mut DbRef) {
        let _ = *stores.get::<i64>(stack);
    }

    /// `client_send(msg)` — everything rides the WebSocket (a browser cannot
    /// UDP); sync sends still count the shared seq space (carrier symmetry).
    pub fn n_kernel_client_send(stores: &mut Stores, stack: &mut DbRef) {
        let msg = stores.get::<Str>(stack).str().to_owned();
        let sync = is_sync_msg(&msg);
        let ok = with_client(|c| {
            if !c.alive {
                return false;
            }
            if sync {
                c.out_seq += 1;
            }
            if !c.opened {
                c.outbox.push(msg.clone());
                return true; // queued; flushed on open (ordered carrier)
            }
            host_ws_send(c.ws, &msg, false) == 1
        })
        .unwrap_or(false);
        stores.put(stack, ok);
    }

    pub fn n_kernel_client_sync_next(stores: &mut Stores, stack: &mut DbRef) {
        let got = with_client(|c| {
            for slot in &mut c.slots {
                if slot.dirty {
                    slot.dirty = false;
                    c.last_sync = (slot.seq, slot.payload.clone());
                    return true;
                }
            }
            false
        })
        .unwrap_or(false);
        stores.put(stack, got);
    }

    pub fn n_kernel_client_sync_seq(stores: &mut Stores, stack: &mut DbRef) {
        let v = with_client(|c| c.last_sync.0).unwrap_or(-1);
        stores.put(stack, v);
    }

    pub fn n_kernel_client_sync_payload_dest(stores: &mut Stores, stack: &mut DbRef) {
        let dest = *stores.get::<DbRef>(stack);
        let v = with_client(|c| c.last_sync.1.clone()).unwrap_or_default();
        stores
            .store_mut(&dest)
            .addr_mut::<String>(dest.rec, dest.pos)
            .push_str(&v);
    }

    /// A browser never has a UDP path — the honest stub.
    pub fn n_kernel_client_udp_bound(stores: &mut Stores, stack: &mut DbRef) {
        stores.put(stack, false);
    }

    /// The browser's per-turn yield: hand the tab back to the event loop;
    /// the host resumes the session next animation frame.
    pub fn n_kernel_client_frame(stores: &mut Stores, _stack: &mut DbRef) {
        stores.frame_yield = true;
    }

    /// `default_host()` in the browser = the cabinet that served the page.
    pub fn n_kernel_default_host_dest(stores: &mut Stores, stack: &mut DbRef) {
        let dest = *stores.get::<DbRef>(stack);
        let v = host_origin_host();
        stores
            .store_mut(&dest)
            .addr_mut::<String>(dest.rec, dest.pos)
            .push_str(&v);
    }
}

// ── Typed twins for `--native` codegen (@PLN18 08 scenario S1) ──────────────
//
// A compiled loft program calls natives by NAME with TYPED Rust signatures
// (the codegen_runtime convention: `cell: &UnsafeCell<Stores>`, `i64`
// scalars, `&str` text args, `String` text returns, `u8` booleans).  These
// twins share the kernel internals with the bytecode-stack natives above —
// one implementation, two calling conventions; the queue machinery never
// forks.  Re-exported by `codegen_runtime` (glob-imported into generated
// crates) and registered in `CODEGEN_RUNTIME_FNS`.
#[cfg(not(target_arch = "wasm32"))]
#[allow(non_snake_case, clippy::missing_panics_doc)]
pub mod typed {
    #[allow(clippy::wildcard_imports)] // the twins mirror the whole module
    use super::*;
    use std::cell::UnsafeCell;

    // ── Listener role ──
    pub fn n_kernel_listen(_cell: &UnsafeCell<Stores>, port: i64, tick_us: i64) -> u8 {
        u8::from(listen_impl(port, tick_us))
    }
    pub fn n_kernel_pump(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(pump_kernel).unwrap_or(0)
    }
    pub fn n_kernel_next_event(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(
            with_kernel(|k| match k.events.pop_front() {
                Some(ev) => {
                    k.last = ev;
                    true
                }
                None => false,
            })
            .unwrap_or(false),
        )
    }
    pub fn n_kernel_event_cid(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.last.cid).unwrap_or(-1)
    }
    pub fn n_kernel_event_kind(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.last.kind).unwrap_or(-1)
    }
    pub fn n_kernel_event_payload(_cell: &UnsafeCell<Stores>) -> String {
        with_kernel(|k| k.last.payload.clone()).unwrap_or_default()
    }
    pub fn n_kernel_tick_due(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_kernel(tick_due_kernel).unwrap_or(false))
    }
    pub fn n_send(_cell: &UnsafeCell<Stores>, cid: i64, msg: &str) -> u8 {
        let sync = is_sync_msg(msg);
        u8::from(with_kernel(|k| deliver(k, cid as usize, msg, sync)).unwrap_or(false))
    }
    pub fn n_broadcast(_cell: &UnsafeCell<Stores>, msg: &str) -> i64 {
        let sync = is_sync_msg(msg);
        with_kernel(|k| {
            let mut sent = 0i64;
            for cid in 0..k.conns.len() {
                if k.conns[cid].is_none() {
                    continue;
                }
                if deliver(k, cid, msg, sync) {
                    sent += 1;
                }
            }
            sent
        })
        .unwrap_or(0)
    }
    pub fn n_kernel_idle(_cell: &UnsafeCell<Stores>, max_us: i64) {
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
    pub fn n_clients(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.conns.iter().filter(|c| c.is_some()).count() as i64).unwrap_or(0)
    }
    pub fn n_udp_bound(_cell: &UnsafeCell<Stores>, cid: i64) -> u8 {
        u8::from(
            with_kernel(|k| k.net.get(cid as usize).is_some_and(|n| n.path.is_some()))
                .unwrap_or(false),
        )
    }
    pub fn n_sync_class(_cell: &UnsafeCell<Stores>, msg_id: i64) {
        SYNC_IDS.with(|s| {
            s.borrow_mut().insert(msg_id, false);
        });
    }
    pub fn n_sync_class_keyed(_cell: &UnsafeCell<Stores>, msg_id: i64) {
        SYNC_IDS.with(|s| {
            s.borrow_mut().insert(msg_id, true);
        });
    }
    pub fn n_keyframe(_cell: &UnsafeCell<Stores>, cid: i64, msg: &str) -> u8 {
        u8::from(with_kernel(|k| deliver_keyframe(k, cid as usize, msg)).unwrap_or(false))
    }
    pub fn n_sync_next(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_kernel(sync_next_kernel).unwrap_or(false))
    }
    pub fn n_sync_cid(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.last_sync.0).unwrap_or(-1)
    }
    pub fn n_sync_seq(_cell: &UnsafeCell<Stores>) -> i64 {
        with_kernel(|k| k.last_sync.1).unwrap_or(-1)
    }
    pub fn n_kernel_sync_payload(_cell: &UnsafeCell<Stores>) -> String {
        with_kernel(|k| k.last_sync.2.clone()).unwrap_or_default()
    }
    pub fn n_kernel_client_frame(_cell: &UnsafeCell<Stores>) {}
    pub fn n_kernel_default_host(_cell: &UnsafeCell<Stores>) -> String {
        std::env::var("LOFT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
    }

    // ── Connector role ──
    pub fn n_kernel_connect(_cell: &UnsafeCell<Stores>, host: &str, port: i64, tick_us: i64) -> u8 {
        u8::from(client_connect(host, port as u16, tick_us).is_some())
    }
    pub fn n_kernel_client_pump(_cell: &UnsafeCell<Stores>) -> i64 {
        with_client(pump_client).unwrap_or(0)
    }
    pub fn n_kernel_client_alive(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_client(|c| c.alive).unwrap_or(false))
    }
    pub fn n_kernel_client_next_event(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(
            with_client(|c| match c.events.pop_front() {
                Some(ev) => {
                    c.last = ev;
                    true
                }
                None => false,
            })
            .unwrap_or(false),
        )
    }
    pub fn n_kernel_client_event_kind(_cell: &UnsafeCell<Stores>) -> i64 {
        with_client(|c| c.last.kind).unwrap_or(-1)
    }
    pub fn n_kernel_client_event_payload(_cell: &UnsafeCell<Stores>) -> String {
        with_client(|c| c.last.payload.clone()).unwrap_or_default()
    }
    pub fn n_kernel_client_tick_due(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_client(tick_due_client).unwrap_or(false))
    }
    pub fn n_kernel_client_idle(_cell: &UnsafeCell<Stores>, max_us: i64) {
        let sleep_us = with_client(|c| {
            let now = c.now_us();
            let until_tick = if c.last_tick_us == 0 {
                c.tick_interval_us
            } else {
                (c.last_tick_us + c.tick_interval_us - now).max(0)
            };
            max_us.clamp(0, until_tick.max(1))
        })
        .unwrap_or(max_us.max(0));
        std::thread::sleep(Duration::from_micros(sleep_us as u64));
    }
    pub fn n_client_send(_cell: &UnsafeCell<Stores>, msg: &str) -> u8 {
        let sync = is_sync_msg(msg);
        u8::from(with_client(|c| client_send_impl(c, msg, sync)).unwrap_or(false))
    }
    pub fn n_client_sync_next(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_client(sync_next_client).unwrap_or(false))
    }
    pub fn n_client_sync_seq(_cell: &UnsafeCell<Stores>) -> i64 {
        with_client(|c| c.last_sync.0).unwrap_or(-1)
    }
    pub fn n_kernel_client_sync_payload(_cell: &UnsafeCell<Stores>) -> String {
        with_client(|c| c.last_sync.1.clone()).unwrap_or_default()
    }
    pub fn n_client_udp_bound(_cell: &UnsafeCell<Stores>) -> u8 {
        u8::from(with_client(|c| c.udp_bound).unwrap_or(false))
    }
}
