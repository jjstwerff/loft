// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN16 M5e slice 1 — the **`--serve` foundation**: a local HTTP + WebSocket server
//! that puts the debugger's wire protocol ([`crate::rpc`]) in a browser.  `GET /` returns
//! a minimal shell (the file's source + a **Run** button + a **Program** console); the
//! shell opens a WebSocket and drives the *same* protocol the `--rpc` stdio server does —
//! one JSON request per text frame, one engine method per request ([`crate::rpc::handle`]).
//! Program `print` rides the same capture sink as `--rpc` ([`crate::rpc::print_or_capture`])
//! and streams back as `output` events, so it never collides with the protocol traffic.
//!
//! The transport is the only new thing here: the engine, the protocol, and the message
//! set are unchanged (PROTOCOL.md's invariant — "one protocol, many transports").  Later
//! M5e slices add panels (compiler console, editor, gutter breakpoints) on the same shell
//! and messages; the live game loop (slice 6) is why this is a WebSocket, not request/
//! response — it needs bidirectional server-push a `POST` round-trip can't give.
//!
//! Scope (slice 1): one browser, one debug session, served single-threaded (each
//! connection handled to completion).  Bound to `127.0.0.1` — the data and the engine stay
//! on the machine, reached remotely only via an explicit SSH port-forward (the `make view`
//! shape).

use crate::repl::ReplSession;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

/// Run the browser debug server on `127.0.0.1:port` for `file`, until the process is
/// killed.  The shell shows `file`'s source and `launch`es it; **Run** executes it and
/// streams its output.
///
/// # Errors
/// Returns an I/O error if the port can't be bound.
pub fn run_serve(stdlib_dir: &str, port: u16, file: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let source = std::fs::read_to_string(file).unwrap_or_default();
    let shell = render_shell(file, &source);
    // One session for the lifetime of the server (slice 1 is single-session); program
    // output is captured into `output` events, not printed, exactly as `--rpc` does.
    let mut session = ReplSession::new(stdlib_dir)?;
    session.debug_stepping(true);
    // Program output is captured into `output` events rather than printed (the same sink
    // `--rpc` uses).  Unlike `--rpc` we do NOT silence the panic hook: the protocol rides
    // the WebSocket, not stdout, so a fault printing to stderr is useful server-side
    // logging, not stream corruption — and the per-request `catch_unwind` already turns it
    // into a `terminated` event for the browser.  (A global hook would also leak across the
    // process, e.g. hiding panic messages in a parallel test run.)
    crate::rpc::capture_begin();
    eprintln!("loft debug --serve: open http://127.0.0.1:{port}/ in a browser");
    for conn in listener.incoming() {
        let Ok(stream) = conn else { continue };
        // A bad connection (malformed request, abrupt close) must not take the server
        // down — drop it and keep serving.
        let _ = serve_connection(&stream, &mut session, &shell);
    }
    crate::rpc::capture_end();
    Ok(())
}

/// Handle one TCP connection: read the request head, then either serve the shell (`GET /`),
/// run the WebSocket protocol loop (an `Upgrade: websocket` request), or 404.
fn serve_connection(
    stream: &TcpStream,
    session: &mut ReplSession,
    shell: &str,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let mut ws_key = None;
    let mut is_upgrade = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break; // end of headers
        }
        if let Some((k, v)) = line.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "upgrade" => is_upgrade = v.trim().eq_ignore_ascii_case("websocket"),
                "sec-websocket-key" => ws_key = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    if is_upgrade && let Some(key) = ws_key {
        let accept = ws_accept_key(&key);
        let resp = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        );
        let mut s = stream;
        s.write_all(resp.as_bytes())?;
        ws_protocol_loop(reader, stream, session)?;
    } else if path == "/" {
        http_respond(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            shell.as_bytes(),
        )?;
    } else {
        http_respond(stream, "404 Not Found", "text/plain", b"not found")?;
    }
    Ok(())
}

/// Write a minimal HTTP/1.1 response and close (no keep-alive — slice 1 serves the shell
/// once, then the WebSocket carries everything).
fn http_respond(stream: &TcpStream, status: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    let mut s = stream;
    s.write_all(head.as_bytes())?;
    s.write_all(body)?;
    Ok(())
}

/// The WebSocket message loop: read one text frame (a JSON request), dispatch it through
/// the shared protocol driver, write each resulting message back as its own text frame.
/// Mirrors `rpc::run_rpc`'s NDJSON loop — same `handle()`, different framing.
fn ws_protocol_loop(
    mut reader: BufReader<&TcpStream>,
    stream: &TcpStream,
    session: &mut ReplSession,
) -> std::io::Result<()> {
    while let Some((opcode, payload)) = ws_read_frame(&mut reader)? {
        match opcode {
            0x1 => {
                // text frame — a JSON request.  Isolate a panic so one bad request can't
                // kill the connection (the engine state is the slice-1 single session).
                let line = String::from_utf8_lossy(&payload).into_owned();
                let (messages, disconnect) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::rpc::handle(session, &line)
                    }))
                    .unwrap_or_else(|_| (vec!["{\"event\":\"terminated\"}".to_string()], false));
                for m in messages {
                    ws_write_text(stream, &m)?;
                }
                if disconnect {
                    break;
                }
            }
            0x8 => break,                            // close
            0x9 => ws_write_pong(stream, &payload)?, // ping → pong
            _ => {} // ignore binary / continuation / pong in slice 1
        }
    }
    Ok(())
}

// ── WebSocket framing (RFC 6455, the subset slice 1 needs) ───────────────────────────

/// Read one WebSocket frame from a client (always masked).  Returns `(opcode, payload)`,
/// or `None` on EOF.  Handles the 7-bit, 16-bit, and 64-bit length forms.
fn ws_read_frame(reader: &mut impl BufRead) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 2];
    if read_exact_or_eof(reader, &mut head)? {
        return Ok(None);
    }
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let len = match head[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            reader.read_exact(&mut b)?;
            u64::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            reader.read_exact(&mut b)?;
            u64::from_be_bytes(b)
        }
        n => u64::from(n),
    };
    let mut mask = [0u8; 4];
    if masked {
        reader.read_exact(&mut mask)?;
    }
    let mut payload = vec![0u8; usize::try_from(len).unwrap_or(0)];
    reader.read_exact(&mut payload)?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok(Some((opcode, payload)))
}

/// True when EOF was hit before filling `buf` (a clean client disconnect).
fn read_exact_or_eof(reader: &mut impl BufRead, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..])? {
            0 => return Ok(true),
            n => filled += n,
        }
    }
    Ok(false)
}

/// Write a server text frame (FIN, opcode 1, unmasked — server frames are never masked).
fn ws_write_text(stream: &TcpStream, text: &str) -> std::io::Result<()> {
    let mut frame = vec![0x81u8]; // FIN + text
    write_frame_len(&mut frame, text.len());
    frame.extend_from_slice(text.as_bytes());
    let mut s = stream;
    s.write_all(&frame)
}

/// Reply to a ping with a pong carrying the same payload (FIN, opcode 0xA, unmasked).
fn ws_write_pong(stream: &TcpStream, payload: &[u8]) -> std::io::Result<()> {
    let mut frame = vec![0x8Au8];
    write_frame_len(&mut frame, payload.len());
    frame.extend_from_slice(payload);
    let mut s = stream;
    s.write_all(&frame)
}

/// Append a server frame's unmasked length (the 7 / 16 / 64-bit forms).
fn write_frame_len(frame: &mut Vec<u8>, len: usize) {
    if len <= 125 {
        frame.push(len as u8);
    } else if len <= 0xFFFF {
        frame.push(126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
}

/// Compute the `Sec-WebSocket-Accept` value: base64(sha1(key + the RFC 6455 GUID)).
fn ws_accept_key(key: &str) -> String {
    const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut input = key.as_bytes().to_vec();
    input.extend_from_slice(GUID.as_bytes());
    crate::base64::encode(&sha1(&input))
}

// ── SHA-1 (inline; the WS handshake's one fixed-length hash — not a security boundary) ──

/// SHA-1 of `data` (RFC 3174).  Inlined rather than pulling a crate for the single
/// handshake hash; covered by the `abc` + RFC 6455 test vectors below.  The single-letter
/// working variables (`a`..`e`, `f`, `k`, `w`, `h`) are the algorithm's own names.
#[allow(clippy::many_single_char_names)]
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            let o = i * 4;
            *word = u32::from_be_bytes([chunk[o], chunk[o + 1], chunk[o + 2], chunk[o + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

// ── the browser shell (slice 1 + 2: line-numbered source, Run, dual console) ─────────

/// Build the IDE shell for `file`: its `source` in a line-numbered read-only pane, a Run
/// button, and the **dual console** — a Compiler pane (diagnostics) over a Program pane
/// (stdout).  The page opens a WebSocket, `launch`es + `compile`s the file on connect, marks
/// diagnostic lines in the source, and on **Run** streams the program's output.  Built by
/// token-replacement (not `format!`) so the CSS/JS braces stay literal.
fn render_shell(file: &str, source: &str) -> String {
    SHELL_TEMPLATE
        .replace("__TITLE__", &html_escape(file))
        .replace("__SOURCE_ROWS__", &render_source_lines(source))
        .replace("__FILE_JS__", &js_string(file))
}

/// Render `source` as line-numbered rows (`<div id="L<n>">`), so a diagnostic at line *n*
/// can mark its row and the gutter shows line numbers.
fn render_source_lines(source: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (i, line) in source.lines().enumerate() {
        let n = i + 1;
        let _ = write!(
            out,
            "<div class=\"row\" id=\"L{n}\"><span class=\"gut\">{n}</span><span class=\"ln\">{}</span></div>",
            html_escape(line)
        );
    }
    out
}

const SHELL_TEMPLATE: &str = r##"<!doctype html><html lang="en"><head><meta charset="utf-8">
<link rel="icon" href="data:,"><title>loft · __TITLE__</title>
<style>
  :root { color-scheme: light dark; }
  body { margin:0; font:14px/1.5 system-ui,sans-serif; }
  #bar { padding:8px 12px; border-bottom:1px solid #8884; display:flex; gap:12px; align-items:center; }
  #bar button { font:inherit; padding:4px 12px; cursor:pointer; }
  #status { opacity:.7; }
  #main { display:grid; grid-template-columns:1fr 1fr; grid-template-rows:1fr 1fr; height:calc(100vh - 41px); }
  #code { grid-row:1 / 3; border-right:1px solid #8884; overflow:auto; padding:8px 0; font:13px/1.45 ui-monospace,monospace; }
  .row { display:flex; white-space:pre; }
  .row .gut { width:3em; text-align:right; padding-right:.8em; opacity:.4; user-select:none; flex:none; }
  .row .ln { flex:1; padding-right:12px; }
  .row.error { box-shadow:inset 3px 0 #e53935; background:#e5393522; }
  .row.warning { box-shadow:inset 3px 0 #f9a825; background:#f9a82522; }
  .pane { display:flex; flex-direction:column; min-height:0; }
  .pane h2 { margin:0; padding:6px 12px; font-size:12px; opacity:.6; border-bottom:1px solid #8884; text-transform:uppercase; letter-spacing:.05em; }
  .body { flex:1; overflow:auto; margin:0; padding:8px 12px; font:13px/1.45 ui-monospace,monospace; white-space:pre-wrap; }
  #diags .d { cursor:pointer; padding:2px 0; }
  #diags .d.error { color:#e53935; } #diags .d.warning { color:#b8860b; }
  #diags .d .loc { opacity:.55; margin-right:.6em; }
  #diags .none { opacity:.45; }
</style></head><body>
<div id="bar"><button id="run">▶ Run</button><span id="status">connecting…</span></div>
<div id="main">
  <div id="code">__SOURCE_ROWS__</div>
  <div class="pane"><h2>Compiler</h2><div id="diags" class="body"></div></div>
  <div class="pane"><h2>Program</h2><pre id="out" class="body"></pre></div>
</div>
<script>
const FILE = __FILE_JS__;
const $ = id => document.getElementById(id);
let ws, mid = 0;
function esc(s) { const d = document.createElement("div"); d.textContent = s; return d.innerHTML; }
function row(n) { return document.getElementById("L" + n); }
function clearDiags() {
  document.querySelectorAll("#code .row.error, #code .row.warning").forEach(r => r.classList.remove("error", "warning"));
  $("diags").innerHTML = "";
}
function showDiags(items) {
  clearDiags();
  if (!items.length) { $("diags").innerHTML = '<div class="none">no problems</div>'; return; }
  for (const it of items) {
    const lvl = (it.level === "error" || it.level === "fatal") ? "error" : "warning";
    const r = row(it.line); if (r) r.classList.add(lvl);
    const d = document.createElement("div");
    d.className = "d " + lvl;
    d.innerHTML = '<span class="loc">' + it.line + ':' + it.col + '</span>' + esc(it.message);
    d.onclick = () => { const rr = row(it.line); if (rr) rr.scrollIntoView({block: "center"}); };
    $("diags").appendChild(d);
  }
}
function connect() {
  ws = new WebSocket("ws://" + location.host + "/ws");
  ws.onopen = () => { $("status").textContent = "connected"; send("launch", {file: FILE}); send("compile", {file: FILE}); };
  ws.onclose = () => { $("status").textContent = "disconnected"; };
  ws.onerror = () => { $("status").textContent = "error"; };
  ws.onmessage = ev => {
    let m; try { m = JSON.parse(ev.data); } catch { return; }
    if (m.event === "diagnostics") showDiags(m.items || []);
    else if (m.event === "output") $("out").textContent += m.text;
    else if (m.event === "stopped") $("out").textContent += "\n⏸ stopped in " + ((m.frame || {}).function || "?") + "\n";
    else if (m.event === "terminated") $("out").textContent += "\n— done —\n";
    else if (m.ok === false && m.error) $("out").textContent += "\n[error] " + m.error + "\n";
  };
}
function send(req, extra) { ws.send(JSON.stringify(Object.assign({id: ++mid, req}, extra))); }
$("run").onclick = () => { if (!ws || ws.readyState !== 1) return; $("out").textContent = ""; send("compile", {file: FILE}); send("run", {}); };
connect();
</script></body></html>"##;

/// HTML-escape `<`, `>`, `&` for embedding text in element content.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render `s` as a JavaScript double-quoted string literal (escaping the few characters
/// that would break out of it).
fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '<' => out.push_str("\\u003c"), // keep out of a `</script>` break
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_vectors() {
        // RFC 3174 "abc".
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        // Empty string.
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn ws_accept_rfc6455_example() {
        // RFC 6455 §1.3 worked example.
        assert_eq!(
            ws_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn frame_len_forms() {
        let mut f = Vec::new();
        write_frame_len(&mut f, 5);
        assert_eq!(f, vec![5]);
        f.clear();
        write_frame_len(&mut f, 200);
        assert_eq!(f, vec![126, 0, 200]);
        f.clear();
        write_frame_len(&mut f, 0x1_0000);
        assert_eq!(f[0], 127);
    }

    #[test]
    fn read_masked_text_frame() {
        // A client text frame "hi", masked — must unmask to "hi".
        let mask = [0xAA, 0xBB, 0xCC, 0xDD];
        let payload = [b'h' ^ mask[0], b'i' ^ mask[1]];
        let mut bytes = vec![0x81, 0x80 | 2]; // FIN+text, masked, len 2
        bytes.extend_from_slice(&mask);
        bytes.extend_from_slice(&payload);
        let mut r = std::io::Cursor::new(bytes);
        let (opcode, data) = ws_read_frame(&mut r).unwrap().unwrap();
        assert_eq!(opcode, 0x1);
        assert_eq!(data, b"hi");
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
