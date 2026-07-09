// @PLN97 — node harness for store_load_url_trusted in wasm (V1b / V3).
//
// Drives the REAL --html wasm through AsyncifyCtrl with a MOCKED
// loft_host_http_get (no network), proving the async→sync asyncify bridge + the
// whole byte flow (fetch -> net::fetch_bytes -> load_url -> validate_structure ->
// adopt) without a browser.  Asserts three cases: a valid image loads; a fetch
// error returns false; a corrupt image returns false (fail-closed).
//
// Usage: node harness.js <urlload.wasm> <world.store>   (run from the repo root)
const fs = require('fs');
const [WASM_PATH, STORE_PATH] = process.argv.slice(2);
if (!WASM_PATH || !STORE_PATH) { console.error('usage: node harness.js <wasm> <store>'); process.exit(2); }
eval(fs.readFileSync('doc/loft-asyncify.js', 'utf8'));   // defines AsyncifyCtrl
const WASM = fs.readFileSync(WASM_PATH);
const STORE = new Uint8Array(fs.readFileSync(STORE_PATH));
const dec = new TextDecoder();

// Run loft_start with a mock fetch returning `mockBytes` (null = simulate a
// network / non-2xx error).  This mock IS the double-call asyncify import the
// real page emits, minus the actual fetch().
async function run(mockBytes) {
  let mem, out = '';
  const ctrl = { ac: null, httpBytes: null };
  const imports = { loft_io: {
    loft_host_print: (p, l) => { out += dec.decode(new Uint8Array(mem.buffer, p, l)); },
    loft_host_input_len: () => 0,
    loft_host_input_copy: () => {},
    loft_host_output: () => {},
    loft_host_http_get: (p, l) => {
      if (ctrl.ac && ctrl.ac.exports.asyncify_get_state() === 2) {   // REWINDING replay
        ctrl.ac.suspend();
        return ctrl.httpBytes ? ctrl.httpBytes.length : 0xFFFFFFFF;
      }
      dec.decode(new Uint8Array(mem.buffer, p, l));                  // url (mock ignores)
      ctrl.httpBytes = null;
      Promise.resolve().then(() => { ctrl.httpBytes = mockBytes; ctrl.ac.resume('loft_start'); });
      ctrl.ac.suspend();                                            // unwind to the event loop
      return 0;
    },
    loft_host_http_get_copy: (p) => { if (ctrl.httpBytes) new Uint8Array(mem.buffer, p, ctrl.httpBytes.length).set(ctrl.httpBytes); },
  } };
  const r = await WebAssembly.instantiate(WASM, imports);
  mem = r.instance.exports.memory;
  ctrl.ac = new AsyncifyCtrl(r.instance);
  ctrl.ac.start('loft_start');
  for (let i = 0; i < 50 && ctrl.ac.sleeping; i++) await new Promise(res => setTimeout(res, 5));
  return out.trim();
}

(async () => {
  const success = await run(STORE);
  const error   = await run(null);
  const corrupt = await run(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]));
  console.log('success :', JSON.stringify(success));
  console.log('error   :', JSON.stringify(error));
  console.log('corrupt :', JSON.stringify(corrupt));
  const pass = success.includes('url keys=7,13,42') && error.includes('FAIL') && corrupt.includes('FAIL');
  console.log(pass ? 'HARNESS PASS' : 'HARNESS FAIL');
  process.exit(pass ? 0 : 1);
})().catch(e => { console.error('harness error:', e); process.exit(2); });
