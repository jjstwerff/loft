#!/usr/bin/env node
// tools/html_asyncify_check.mjs — load a `loft --html` page in headless
// Chrome and assert that an asyncify program RESUMES past its first suspend.
//
// tests/html_render.rs checks that a GL page renders SOME frame without a
// console error, and tests/html_wasm.rs checks that a SYNCHRONOUS wasm bundle
// returns without trapping.  Neither catches issue #450: an asyncify program
// (frame_yield / gl_swap_buffers / ws_yield) that suspends, but never resumes
// past the FIRST suspend.  The buggy resume loop still printed the first line
// and rendered the first frame — so both existing gates passed while every
// asyncify program was stuck on iteration 0.
//
// This harness drives the real generated page (its embedded AsyncifyCtrl +
// resume scheduler) and asserts the program's `#out` text reaches an expected
// marker — i.e. it actually progressed across multiple suspend/resume cycles.
//
// Usage:
//   node tools/html_asyncify_check.mjs <html_file> --expect <substr>
//        [--wait-ms N] [--hidden] [--port N]
//
//   --hidden   Force document.hidden=true AND a dead requestAnimationFrame
//              BEFORE the page runs — the headless / backgrounded-tab
//              condition where rAF is paused (issue #450).  If the page still
//              reaches the marker, only the non-rAF pump drove it.
//
// Exit 0 if `#out` contains the expected substring, 1 if not (prints the
// captured `#out`), 2 = SKIP (no chrome binary).
//
// The file is served over http://localhost so a snap-confined Chromium (which
// cannot read /tmp via file://) can load it.

import http from 'node:http';
import net from 'node:net';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import process from 'node:process';

const argv = process.argv.slice(2);
if (argv.length < 1) {
  console.error('usage: html_asyncify_check.mjs <html_file> --expect <substr> [--wait-ms N] [--hidden] [--port N]');
  process.exit(64);
}
const FILE_ARG = argv[0];
let expect = null, waitMs = 5000, hidden = false, cdpPort = 9555;
for (let i = 1; i < argv.length; i++) {
  if (argv[i] === '--expect') expect = argv[++i];
  else if (argv[i] === '--wait-ms') waitMs = parseInt(argv[++i], 10);
  else if (argv[i] === '--hidden') hidden = true;
  else if (argv[i] === '--port') cdpPort = parseInt(argv[++i], 10);
  else { console.error('unknown flag: ' + argv[i]); process.exit(64); }
}
if (expect === null) { console.error('--expect <substr> is required'); process.exit(64); }

const CHROME_NAMES = ['google-chrome', 'chromium', 'chromium-browser', 'chrome'];
let CHROME = null;
for (const name of CHROME_NAMES) {
  const r = spawnSync('sh', ['-c', `command -v ${name}`], { encoding: 'utf8' });
  if (r.stdout && r.stdout.trim()) { CHROME = r.stdout.trim(); break; }
}
if (CHROME === null) { console.error('SKIP: no chrome binary in PATH'); process.exit(2); }

// Serve the HTML (+ siblings) over localhost.
const serveDir = path.dirname(path.resolve(FILE_ARG));
const serveName = path.basename(FILE_ARG);
const fileServer = http.createServer((req, res) => {
  const name = decodeURIComponent(req.url.split('?')[0].replace(/^\//, '')) || serveName;
  const fp = path.join(serveDir, name);
  if (!fp.startsWith(serveDir)) { res.writeHead(403); res.end(); return; }
  fs.readFile(fp, (err, data) => {
    if (err) { res.writeHead(404); res.end(); return; }
    const ext = path.extname(fp);
    const ct = ext === '.html' ? 'text/html' : ext === '.wasm' ? 'application/wasm' : 'application/octet-stream';
    res.writeHead(200, { 'Content-Type': ct }); res.end(data);
  });
});
const httpPort = cdpPort + 1;
fileServer.listen(httpPort);
const URL_ARG = `http://localhost:${httpPort}/${serveName}`;

const chrome = spawn(CHROME, [
  '--headless=new', '--no-sandbox', '--remote-debugging-port=' + cdpPort,
  '--window-size=800,600', '--enable-unsafe-swiftshader',
  '--use-gl=angle', '--use-angle=swiftshader', '--mute-audio', 'about:blank',
], { stdio: ['ignore', 'pipe', 'pipe'] });
let chromeErr = '';
chrome.stderr.on('data', d => { chromeErr += d.toString(); });

function getJson(p) {
  return new Promise((resolve, reject) => {
    http.get('http://localhost:' + cdpPort + p, r => {
      let body = ''; r.on('data', c => body += c);
      r.on('end', () => { try { resolve(JSON.parse(body)); } catch (e) { reject(e); } });
    }).on('error', reject);
  });
}
async function waitFor(check, maxTries = 50) {
  for (let i = 0; i < maxTries; i++) {
    try { const r = await check(); if (r) return r; } catch (e) {}
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error('timeout waiting for chrome');
}
function encodeFrame(text) {
  const payload = Buffer.from(text, 'utf8');
  const mask = crypto.randomBytes(4);
  const len = payload.length;
  let header;
  if (len < 126) header = Buffer.from([0x81, 0x80 | len]);
  else if (len < 65536) header = Buffer.from([0x81, 0xfe, (len >> 8) & 0xff, len & 0xff]);
  else { header = Buffer.alloc(10); header[0] = 0x81; header[1] = 0xff; header.writeBigUInt64BE(BigInt(len), 2); }
  const masked = Buffer.alloc(payload.length);
  for (let i = 0; i < payload.length; i++) masked[i] = payload[i] ^ mask[i % 4];
  return Buffer.concat([header, mask, masked]);
}

let exitCode = 0;
(async () => {
  try {
    await waitFor(() => getJson('/json/version'));
    const tab = await waitFor(async () => {
      const tabs = await getJson('/json');
      return tabs.find(t => t.type === 'page');
    });
    const u = tab.webSocketDebuggerUrl.match(/^ws:\/\/([^:]+):(\d+)(\/.*)$/);
    const sock = net.createConnection(parseInt(u[2], 10), u[1]);
    await new Promise((res, rej) => { sock.once('connect', res); sock.once('error', rej); });
    const key = crypto.randomBytes(16).toString('base64');
    sock.write(`GET ${u[3]} HTTP/1.1\r\nHost: ${u[1]}:${u[2]}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: ${key}\r\nSec-WebSocket-Version: 13\r\n\r\n`);
    let buf = Buffer.alloc(0);
    await new Promise(res => sock.once('data', d => { const e = d.indexOf('\r\n\r\n'); buf = d.slice(e + 4); res(); }));
    let nextId = 1; const pending = new Map();
    sock.on('data', d => {
      buf = Buffer.concat([buf, d]);
      while (buf.length >= 2) {
        let len = buf[1] & 0x7f, off = 2;
        if (len === 126) { len = buf.readUInt16BE(2); off = 4; }
        else if (len === 127) { len = Number(buf.readBigUInt64BE(2)); off = 10; }
        if (buf.length < off + len) break;
        const text = buf.slice(off, off + len).toString('utf8'); buf = buf.slice(off + len);
        try { const msg = JSON.parse(text); if (msg.id && pending.has(msg.id)) { const cb = pending.get(msg.id); pending.delete(msg.id); cb(msg); } } catch (e) {}
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
    if (hidden) {
      await send('Page.addScriptToEvaluateOnNewDocument', {
        source: `
          Object.defineProperty(document, 'hidden', { get: () => true, configurable: true });
          Object.defineProperty(document, 'visibilityState', { get: () => 'hidden', configurable: true });
          window.requestAnimationFrame = function(){ return 0; };
        `,
      });
    }
    await send('Page.navigate', { url: URL_ARG });
    await new Promise(r => setTimeout(r, waitMs));
    const out = await send('Runtime.evaluate', {
      expression: "document.getElementById('out') ? document.getElementById('out').textContent : '<no #out>'",
      returnByValue: true,
    });
    const text = out.result.value;
    if (typeof text === 'string' && text.includes(expect)) {
      console.log(JSON.stringify({ ok: true, hidden, expect }));
    } else {
      console.error(JSON.stringify({ ok: false, hidden, expect, got: text }, null, 2));
      exitCode = 1;
    }
  } catch (e) {
    console.error('harness error: ' + e.message);
    if (chromeErr) console.error('chrome stderr (last 400):\n' + chromeErr.slice(-400));
    exitCode = 3;
  } finally {
    chrome.kill();
    process.exit(exitCode);
  }
})();
