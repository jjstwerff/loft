// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN84 ZT-C — tiny RFC 6455 WebSocket echo server for the C2/C3 verify
// loop.  Dependency-free (raw `node:net` + `node:crypto`): accepts one
// upgrade, echoes every TEXT and BINARY frame back with the SAME opcode
// (so a binary CBOR frame echoes as binary — load-bearing for C3's
// byte-identity).  Replies PONG to PING.  Used by both the native client
// (real TCP) and the wasm client (driven through host.js's `new
// WebSocket`).
//
// Usage:
//   node ws_echo_server.mjs [port]           # default 9099
// Prints `listening <port>` on stdout once ready (a harness greps for it).

import net from 'node:net';
import crypto from 'node:crypto';
import process from 'node:process';

const PORT = parseInt(process.argv[2] || '9099', 10);
const GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';

function acceptKey(key) {
  return crypto
    .createHash('sha1')
    .update(key + GUID)
    .digest('base64');
}

// Encode an UNMASKED server->client frame (servers never mask).
function encodeFrame(opcode, payload) {
  const len = payload.length;
  let header;
  if (len < 126) {
    header = Buffer.from([0x80 | opcode, len]);
  } else if (len < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(len), 2);
  }
  return Buffer.concat([header, payload]);
}

const server = net.createServer((sock) => {
  let buf = Buffer.alloc(0);
  let upgraded = false;

  sock.on('data', (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    if (!upgraded) {
      const end = buf.indexOf('\r\n\r\n');
      if (end < 0) return;
      const headerText = buf.slice(0, end).toString('ascii');
      buf = buf.slice(end + 4);
      const m = headerText.match(/sec-websocket-key:\s*(.+)\r?/i);
      if (!m) {
        sock.destroy();
        return;
      }
      const resp =
        'HTTP/1.1 101 Switching Protocols\r\n' +
        'Upgrade: websocket\r\n' +
        'Connection: Upgrade\r\n' +
        `Sec-WebSocket-Accept: ${acceptKey(m[1].trim())}\r\n\r\n`;
      sock.write(resp);
      upgraded = true;
    }
    // Decode any complete frames in `buf`.
    while (buf.length >= 2) {
      const b0 = buf[0];
      const b1 = buf[1];
      const opcode = b0 & 0x0f;
      const masked = (b1 & 0x80) !== 0;
      let len = b1 & 0x7f;
      let off = 2;
      if (len === 126) {
        if (buf.length < 4) return;
        len = buf.readUInt16BE(2);
        off = 4;
      } else if (len === 127) {
        if (buf.length < 10) return;
        len = Number(buf.readBigUInt64BE(2));
        off = 10;
      }
      const maskOff = off;
      const dataOff = masked ? off + 4 : off;
      if (buf.length < dataOff + len) return; // wait for the rest
      let payload = buf.slice(dataOff, dataOff + len);
      if (masked) {
        const mask = buf.slice(maskOff, maskOff + 4);
        const out = Buffer.alloc(len);
        for (let i = 0; i < len; i++) out[i] = payload[i] ^ mask[i % 4];
        payload = out;
      }
      buf = buf.slice(dataOff + len);

      if (opcode === 0x1 || opcode === 0x2) {
        // Echo back with the same opcode (text=1 / binary=2).
        sock.write(encodeFrame(opcode, payload));
      } else if (opcode === 0x9) {
        sock.write(encodeFrame(0xa, payload)); // PING -> PONG
      } else if (opcode === 0x8) {
        sock.write(encodeFrame(0x8, Buffer.alloc(0))); // CLOSE echo
        sock.end();
      }
    }
  });
  sock.on('error', () => sock.destroy());
});

server.listen(PORT, '127.0.0.1', () => {
  process.stdout.write(`listening ${PORT}\n`);
});
