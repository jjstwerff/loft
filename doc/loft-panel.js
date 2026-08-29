// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN149 step 8 — the Run / REPL / Debug panel on a documentation page.
//
// Every executed topic page IS a loft program, and until now the page could only show it.
// This drives it, in the reader's own browser, through two wasm entries:
//
//   debug_start(source)  -> {"ok":true} | {"ok":false,"error":"…"}
//   debug_command(cmd)   -> {"replies":[…],"output":"…"}
//
// The command grammar is the debugger's own (`src/wasm_debug.rs`), so this page and the
// `--html --debug` relay drive one implementation.
//
// One constraint shapes the whole surface: `eval` reads its answer off a LIVE FRAME, so a
// prompt against a finished program has nothing to evaluate against. Run therefore ends
// paused — on the reader's breakpoint if they set one, otherwise on the last line of
// `main` (`bp end`), where every local main assigned is still standing. The panel says
// which of the two it is, rather than presenting a calculator that mysteriously cannot see
// the program's variables.

import init, { debug_start, debug_command } from './pkg/loft.js';
import { createHost } from './loft-rt.js';

const $ = (id) => document.getElementById(id);

// The page's own source, emitted beside the panel by gendoc as a hidden `<pre>` — not a
// `<script>`, whose content is raw text where an escaped `&lt;` would stay four characters
// and the panel would compile something other than the program on the page.
const sourceEl = $('lp-source');
const SOURCE = sourceEl ? sourceEl.textContent : '';

let ready = false;
// The line the reader asked to stop on, or null for the end-of-main pause.
let breakLine = null;

function setStatus(text, kind) {
  const el = $('lp-status');
  el.textContent = text;
  el.className = 'lp-status' + (kind ? ' lp-' + kind : '');
}

function log(entry, value, kind) {
  const row = document.createElement('div');
  row.className = 'lp-row' + (kind ? ' lp-' + kind : '');
  const q = document.createElement('code');
  q.className = 'lp-q';
  q.textContent = entry;
  const a = document.createElement('code');
  a.className = 'lp-a';
  a.textContent = value;
  row.append(q, a);
  $('lp-log').append(row);
  row.scrollIntoView({ block: 'nearest' });
}

function send(cmd) {
  const r = JSON.parse(debug_command(cmd));
  if (r.output) {
    $('lp-output').textContent += r.output;
  }
  return r.replies;
}

// `D:hit <fn> a=1|b=2` — the pause, its function and the locals worth showing.
function renderPause(reply) {
  const rest = reply.slice('D:hit '.length);
  const sp = rest.indexOf(' ');
  const fn = sp < 0 ? rest : rest.slice(0, sp);
  const locals = sp < 0 ? '' : rest.slice(sp + 1);
  const box = $('lp-frame');
  box.innerHTML = '';
  const head = document.createElement('div');
  head.className = 'lp-frame-head';
  head.textContent = breakLine === null
    ? 'paused just before main\u2019s last line, so its variables are still live and the '
      + 'prompt can see them \u2014 Resume lets the program finish'
    : `paused in ${fn} at line ${breakLine}`;
  box.append(head);
  const list = document.createElement('div');
  list.className = 'lp-locals';
  for (const item of locals ? locals.split('|') : []) {
    const eq = item.indexOf('=');
    if (eq < 0) continue;
    const chip = document.createElement('code');
    chip.className = 'lp-local';
    chip.textContent = item;
    chip.title = 'click to put this name in the prompt';
    chip.addEventListener('click', () => {
      $('lp-input').value = item.slice(0, eq);
      $('lp-input').focus();
    });
    list.append(chip);
  }
  if (!list.children.length) {
    const none = document.createElement('span');
    none.className = 'lp-note';
    none.textContent = 'no variables in scope at this line yet';
    list.append(none);
  }
  box.append(list);
  box.hidden = false;
  $('lp-input').disabled = false;
  $('lp-step').disabled = false;
  $('lp-resume').disabled = false;
  setStatus('paused — type an expression below', 'ok');
}

function renderDone() {
  $('lp-frame').hidden = true;
  $('lp-input').disabled = true;
  $('lp-step').disabled = true;
  $('lp-resume').disabled = true;
  setStatus('finished — press Run to start again', 'info');
}

function applyReplies(replies) {
  for (const r of replies) {
    if (r.startsWith('D:hit ')) {
      renderPause(r);
    } else if (r === 'D:terminated') {
      renderDone();
    }
  }
}

function run() {
  if (!ready) return;
  $('lp-output').textContent = '';
  $('lp-log').innerHTML = '';
  $('lp-frame').hidden = true;
  setStatus('running…', 'info');

  const started = JSON.parse(debug_start(SOURCE));
  if (!started.ok) {
    setStatus('this program does not compile in the browser', 'err');
    $('lp-output').textContent = started.error || '';
    return;
  }
  // A breakpoint the reader set, or the end-of-main pause that gives the prompt a frame.
  send(breakLine === null ? 'bp end' : 'bp ' + breakLine);
  applyReplies(send('run'));
  listCallables();
}

// The program's own functions, off the session rather than off a hardcoded list, so the
// panel names what this page actually defines.
function listCallables() {
  const reply = send('fns')[0] || '';
  const body = reply.startsWith('D:fns ') ? reply.slice('D:fns '.length) : '';
  const box = $('lp-callables');
  box.innerHTML = '';
  const names = body ? body.split('|').filter(Boolean) : [];
  if (!names.length) {
    box.hidden = true;
    return;
  }
  const lead = document.createElement('span');
  lead.className = 'lp-note';
  lead.textContent = 'this page defines: ';
  box.append(lead);
  for (const sig of names) {
    const name = sig.slice(0, sig.indexOf('('));
    const chip = document.createElement('code');
    chip.className = 'lp-callable';
    chip.textContent = sig;
    chip.title = 'click to start a call to ' + name;
    chip.addEventListener('click', () => {
      $('lp-input').value = name + '(';
      $('lp-input').focus();
    });
    box.append(chip);
  }
  box.hidden = false;
}

function evaluate(expr) {
  const replies = send('eval ' + expr);
  const r = replies[0] || '';
  const prefix = 'D:eval ' + expr + '=';
  const value = r.startsWith(prefix) ? r.slice(prefix.length) : r;
  if (value === '<unavailable>') {
    // Do not let this read as a typo. The evaluator reads its answer off the paused
    // frame, and a text or vector result does not survive that trip (loft#1187).
    log(expr, 'no value — a text or vector result cannot be read from the browser yet, '
      + 'and a name that is not in scope here has none either', 'warn');
  } else {
    log(expr, value);
  }
}

// The source, one clickable line per row: a click sets the breakpoint the next Run uses.
function buildLines() {
  const ol = $('lp-src');
  ol.innerHTML = '';
  SOURCE.split('\n').forEach((text, i) => {
    const li = document.createElement('li');
    li.className = 'lp-line';
    li.textContent = text === '' ? ' ' : text;
    li.addEventListener('click', () => {
      const line = i + 1;
      breakLine = breakLine === line ? null : line;
      for (const el of ol.children) el.classList.remove('lp-break');
      if (breakLine !== null) li.classList.add('lp-break');
      setStatus(
        breakLine === null
          ? 'breakpoint cleared — Run will pause at the end of main'
          : `breakpoint on line ${breakLine} — press Run`,
        'info',
      );
    });
    ol.append(li);
  });
}

async function boot() {
  const panel = $('loft-panel');
  if (!panel || !SOURCE) return;
  panel.hidden = false;
  buildLines();

  $('lp-run').addEventListener('click', run);
  $('lp-step').addEventListener('click', () => applyReplies(send('step')));
  $('lp-resume').addEventListener('click', () => applyReplies(send('resume')));
  $('lp-input').addEventListener('keydown', (e) => {
    if (e.key !== 'Enter') return;
    const expr = e.target.value.trim();
    if (!expr) return;
    e.target.value = '';
    evaluate(expr);
  });

  try {
    // The same host shim the playground installs: the wasm side calls
    // `globalThis.loftHost.*` for file, time, random and log operations.
    const { host } = createHost();
    window.loftHost = host;
    await init();
    ready = true;
    $('lp-run').disabled = false;
    setStatus('ready — press Run', 'ok');
  } catch (e) {
    setStatus('the loft runtime did not load: ' + e.message, 'err');
  }
}

boot();
