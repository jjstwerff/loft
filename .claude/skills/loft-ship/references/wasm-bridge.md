# The Tier-2 `wasm.bridge` recipe

Read this when a `#native` library must run in the **browser** (`--html`) — the one matrix
cell with no automatic path. The bridge lets the library's wasm call a **host capability**
(crypto, sockets, the DOM, files) that wasm itself can't reach. Full reference:
[PACKAGES.md § library-owned wasm bridges](../../../../doc/claude/PACKAGES.md),
[WASM.md](../../../../doc/claude/WASM.md), [HTML_EXPORT.md](../../../../doc/claude/HTML_EXPORT.md).

## The shape (four parts)

```
my_lib/
├── loft.toml            # [wasm.bridge] block names the crate + the host module
├── src/my_lib.loft      # the loft API; #native fns map to host imports
└── wasm/                # the bridge crate
    └── src/lib.rs        # imports `loft_<lib>.<fn>`; marshals the CBOR ABI
```

1. **`[wasm.bridge]` in `loft.toml`** — declares the bridge crate and the host-import module
   name (`loft_<lib>`, e.g. `loft_web`, `loft_crypto`). The `loft_`-prefix is what the runtime
   recognizes as a permitted host-import module.
2. **The bridge crate (`wasm/`)** — a small Rust crate compiled to wasm alongside the library.
   Each `#native` function becomes an `extern` import `loft_<lib>.<fn>` that the crate calls
   and whose CBOR-encoded args/results it marshals.
3. **The host shim** — the JS/WASI side that *implements* those imports:
   - **Browser (`--html`):** a `host.js` providing `loft_<lib>.<fn>` against the real
     capability (WebCrypto, WebSocket, the DOM). It's wired into the page's import object next
     to the loft runtime imports.
   - **Headless wasm (`--native-wasm`):** a WASI host shim (a Node/wasmtime driver) providing
     the same imports — used by the parity gate so you can test the bridge without a browser.
4. **The CBOR ABI** — args and results cross the boundary CBOR-encoded. This is the silent-
   corruption surface; see the traps.

## Asyncify — the suspend trap (this one cost real time)

If a bridge function **yields or awaits** (a socket read, a frame yield, anything async on the
host), the wasm must be asyncify-transformed so it can suspend and resume:

```bash
wasm-opt --asyncify --pass-arg=asyncify-imports@loft_<lib>.<suspending_fn>[,...] in.wasm -o out.wasm
```

The trap that bites: **`yield_frame()` only sets a flag — it does NOT itself suspend.** Only an
import listed in `--pass-arg=asyncify-imports@…` actually unwinds the stack. So a suspending
call needs a **dedicated suspend import** (the @PLN84 pattern: `loft_web.ws_yield`), added to
the asyncify import list — not a reuse of a flag-only yield. If your "await" returns instantly
or hangs, this is almost always why.

## The CBOR ABI traps

- **Validate with a round-trip *value* check, not "it didn't crash."** A mis-sized or mis-
  ordered field decodes to garbage that often *looks* plausible — assert the decoded value
  equals the sent value, on a distinctive payload.
- **The control struct has its own ABI.** The asyncify control block (the AsyncifyCtrl
  layout — stack ptr / data ptrs) is part of the contract; a wrong offset there corrupts the
  suspend/resume, not the payload, so it presents as a hang or a wild pointer, not a decode
  error. Two such bugs hid in the @PLN84 WS bridge — suspect the control ABI when the *value*
  round-trips but the *suspend* misbehaves.

## The gate for a bridge

The bridge is done only when the library passes the **parity gate** on `--native-wasm` (via
the WASI host shim) **and** `--html` (via `host.js`), with results equal to `--interpret`. The
headless WASI driver exists precisely so you can prove `--native-wasm` parity in CI without a
browser; build/keep one (the @PLN84 `wasm_ws_repro.mjs` is the model).
