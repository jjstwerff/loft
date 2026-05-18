#!/usr/bin/env node
// tools/html_render_check.mjs — load an HTML page in headless Chrome
// + capture JS console events / exceptions via the Chrome DevTools
// Protocol.  Exit code 0 if no console errors / exceptions.  Exit
// code 1 with a JSON summary on stderr otherwise.
//
// Used by tests/html_render.rs to gate browser-WebGL rendering.  The
// recurring failure mode for `--html` artefacts (shader compile errors,
// canvas-init exceptions, missing WebGL2 contexts) is a JS-side
// console.error — invisible to the WASM-side success/failure signal
// that tests/html_wasm.rs uses.  This harness wires the CDP socket
// directly so those events become a hard fail.
//
// Usage:
//   node tools/html_render_check.mjs <url> [--wait-ms N] [--screenshot path]
//                                          [--port N]   [--allow PATTERN]
//
// Skip protocol — if google-chrome is not installed, exit 2 with a
// `SKIP: …` line on stderr (callers turn this into a test skip).

import http from 'node:http';
import net from 'node:net';
import crypto from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import process from 'node:process';

// ── CLI ────────────────────────────────────────────────────────────
const args = process.argv.slice(2);
if (args.length < 1) {
  console.error('usage: html_render_check.mjs <url> [--wait-ms N] [--screenshot PATH] [--port N] [--allow REGEX]');
  process.exit(64);
}
const URL_ARG = args[0];
const opts = { waitMs: 8000, screenshot: null, port: 9222, allow: [] };
for (let i = 1; i < args.length; i++) {
  if (args[i] === '--wait-ms') opts.waitMs = parseInt(args[++i], 10);
  else if (args[i] === '--screenshot') opts.screenshot = args[++i];
  else if (args[i] === '--port') opts.port = parseInt(args[++i], 10);
  else if (args[i] === '--allow') opts.allow.push(new RegExp(args[++i]));
  else { console.error('unknown flag: ' + args[i]); process.exit(64); }
}

// ── Chrome availability ────────────────────────────────────────────
const CHROME_NAMES = ['google-chrome', 'chromium', 'chromium-browser', 'chrome'];
let CHROME = null;
for (const name of CHROME_NAMES) {
  const r = spawnSync('sh', ['-c', `command -v ${name}`], { encoding: 'utf8' });
  if (r.stdout && r.stdout.trim()) { CHROME = r.stdout.trim(); break; }
}
if (!CHROME) {
  console.error('SKIP: no chrome binary in PATH (tried: ' + CHROME_NAMES.join(', ') + ')');
  process.exit(2);
}

// ── Spawn Chrome ───────────────────────────────────────────────────
// Headless WebGL2 requires SwiftShader (no GPU in CI).  The Vulkan
// feature flag silences a Chrome 121+ regression that disables WebGL2
// when no GPU is present without the explicit opt-in.
const chrome = spawn(CHROME, [
  '--headless=new',
  '--no-sandbox',
  '--remote-debugging-port=' + opts.port,
  '--window-size=1024,768',
  '--enable-unsafe-swiftshader',
  '--use-gl=angle',
  '--use-angle=swiftshader',
  '--disable-features=Vulkan',
  '--mute-audio',
  '--hide-scrollbars',
  'about:blank',
], { stdio: ['ignore', 'pipe', 'pipe'] });

let chromeErr = '';
chrome.stderr.on('data', d => { chromeErr += d.toString(); });
chrome.on('exit', code => {
  if (code !== null && code !== 0 && !chrome._intentionalExit) {
    console.error('chrome exited unexpectedly: code=' + code + '\n' + chromeErr.slice(-500));
  }
});

// ── Helpers ────────────────────────────────────────────────────────
function getJson(path) {
  return new Promise((resolve, reject) => {
    http.get('http://localhost:' + opts.port + path, r => {
      let body = '';
      r.on('data', c => body += c);
      r.on('end', () => { try { resolve(JSON.parse(body)); } catch (e) { reject(e); } });
    }).on('error', reject);
  });
}
function putJson(path) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: 'localhost', port: opts.port, method: 'PUT', path }, r => {
      let body = '';
      r.on('data', c => body += c);
      r.on('end', () => { try { resolve(JSON.parse(body)); } catch (e) { reject(e); } });
    });
    req.on('error', reject); req.end();
  });
}
async function waitFor(check, maxTries = 50) {
  for (let i = 0; i < maxTries; i++) {
    try { const r = await check(); if (r) return r; } catch (e) {}
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error('timeout waiting for ' + check.toString().slice(0, 80));
}

// ── WebSocket (minimal RFC6455 client) ─────────────────────────────
function parseWsUrl(url) {
  const m = url.match(/^ws:\/\/([^:]+):(\d+)(\/.*)$/);
  if (!m) throw new Error('bad ws url: ' + url);
  return { host: m[1], port: parseInt(m[2], 10), path: m[3] };
}
async function openWebSocket(url) {
  const u = parseWsUrl(url);
  const sock = net.createConnection(u.port, u.host);
  await new Promise((resolve, reject) => {
    sock.once('connect', resolve);
    sock.once('error', reject);
  });
  const key = crypto.randomBytes(16).toString('base64');
  sock.write(
    `GET ${u.path} HTTP/1.1\r\nHost: ${u.host}:${u.port}\r\n` +
    `Upgrade: websocket\r\nConnection: Upgrade\r\n` +
    `Sec-WebSocket-Key: ${key}\r\nSec-WebSocket-Version: 13\r\n\r\n`
  );
  let buf = Buffer.alloc(0);
  await new Promise(resolve => sock.once('data', d => { buf = d; resolve(); }));
  const headerEnd = buf.indexOf('\r\n\r\n');
  buf = buf.slice(headerEnd + 4);
  return { sock, buf };
}
function encodeFrame(text) {
  const payload = Buffer.from(text, 'utf8');
  const mask = crypto.randomBytes(4);
  const len = payload.length;
  let header;
  if (len < 126) {
    header = Buffer.from([0x81, 0x80 | len]);
  } else if (len < 65536) {
    header = Buffer.from([0x81, 0xfe, (len >> 8) & 0xff, len & 0xff]);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x81; header[1] = 0xff;
    header.writeBigUInt64BE(BigInt(len), 2);
  }
  const masked = Buffer.alloc(payload.length);
  for (let i = 0; i < payload.length; i++) masked[i] = payload[i] ^ mask[i % 4];
  return Buffer.concat([header, mask, masked]);
}

// ── Main ───────────────────────────────────────────────────────────
(async () => {
  let exitCode = 0;
  try {
    await waitFor(() => getJson('/json/version'));
    const tab = await putJson('/json/new?' + encodeURIComponent(URL_ARG));
    const wsState = await openWebSocket(tab.webSocketDebuggerUrl);
    let { sock } = wsState;
    let buf = wsState.buf;
    let nextId = 1;
    const pending = new Map();
    const events = [];
    sock.on('data', d => {
      buf = Buffer.concat([buf, d]);
      while (buf.length >= 2) {
        let len = buf[1] & 0x7f;
        let off = 2;
        if (len === 126) { len = buf.readUInt16BE(2); off = 4; }
        else if (len === 127) { len = Number(buf.readBigUInt64BE(2)); off = 10; }
        if (buf.length < off + len) break;
        const text = buf.slice(off, off + len).toString('utf8');
        buf = buf.slice(off + len);
        try {
          const msg = JSON.parse(text);
          if (msg.id && pending.has(msg.id)) {
            const cb = pending.get(msg.id); pending.delete(msg.id); cb(msg);
          } else if (msg.method) {
            events.push(msg);
          }
        } catch (e) {}
      }
    });
    function send(method, params = {}) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        pending.set(id, msg => msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result));
        sock.write(encodeFrame(JSON.stringify({ id, method, params })));
      });
    }

    await send('Runtime.enable');
    await send('Page.enable');
    await send('Log.enable');
    await send('Page.navigate', { url: URL_ARG });
    await new Promise(r => setTimeout(r, opts.waitMs));

    // Optional screenshot
    if (opts.screenshot) {
      try {
        const shot = await send('Page.captureScreenshot', { format: 'png' });
        fs.writeFileSync(opts.screenshot, Buffer.from(shot.data, 'base64'));
      } catch (e) {
        console.error('screenshot failed: ' + e.message);
      }
    }

    // Collect failures
    const failures = [];
    for (const e of events) {
      if (e.method === 'Runtime.consoleAPICalled' && e.params.type === 'error') {
        const text = e.params.args.map(a => a.value ?? a.description ?? '').join(' ');
        if (!opts.allow.some(r => r.test(text))) {
          failures.push({ kind: 'console.error', text });
        }
      } else if (e.method === 'Runtime.exceptionThrown') {
        const text = e.params.exceptionDetails.exception?.description ||
                     e.params.exceptionDetails.text || 'unknown exception';
        if (!opts.allow.some(r => r.test(text))) {
          failures.push({ kind: 'exception', text });
        }
      } else if (e.method === 'Log.entryAdded' && e.params.entry.level === 'error') {
        const text = e.params.entry.text;
        // Browser-side resource 404s (favicon, etc.) are noise unless
        // they hit a file we intentionally load.  Filter the generic
        // "Failed to load resource" pattern unless --allow excludes it.
        if (/Failed to load resource/.test(text) && !opts.allow.some(r => r.test(text))) {
          continue;
        }
        if (!opts.allow.some(r => r.test(text))) {
          failures.push({ kind: 'log.error', text });
        }
      }
    }

    if (failures.length > 0) {
      console.error(JSON.stringify({ ok: false, url: URL_ARG, failures }, null, 2));
      exitCode = 1;
    } else {
      console.log(JSON.stringify({ ok: true, url: URL_ARG, eventCount: events.length }));
    }
  } catch (e) {
    console.error('harness error: ' + e.message);
    if (chromeErr) console.error('chrome stderr (last 500):\n' + chromeErr.slice(-500));
    exitCode = 3;
  } finally {
    chrome._intentionalExit = true;
    chrome.kill();
  }
  process.exit(exitCode);
})();
