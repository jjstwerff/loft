<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# NEXT SESSION — resume the text-tail-return leak (@PLN85 reopen / @PLN54 S4)

One-screen handoff. Full detail: [text-tail-return-leak.md](text-tail-return-leak.md).

## Branch / state (2026-07-09)

- Branch **`mac-work`**, fully pushed (`origin/mac-work` = `3c8eb408`), tree clean.
  Stacked on `origin/main`'s `2b38b34d` (the #539 PLN54 squash). 15 commits this
  session; the two CODE fixes are `dd279705` (attempt 2) and `f5037347` + `5715292e`
  (attempt 2d + return-peel). Everything else is docs/probes.
- Runs on a **macOS-ARM** box. Local sanitizer invocation quirks (no rustup proxy;
  LSan works on ARM) are in the `mac_sanitizer_toolchain` private memory.

## What's DONE (landed, validated on BOTH backends, 749/0)

- **@PLN54 S1** — macOS-ARM Miri+ASan matrix leg in `miri.yml` (validated on ARM).
- **@PLN54 S4** — LSan baseline root-caused: two classes — (1) intentional `ir_read`
  `Box::leak` (interner); (2) a REAL growing production leak = the text-tail-return
  class below.
- **text-tail-return leak** — a native text-dest CALL delivered as a fn's return
  value leaked (and `-> text?` was a USE-AFTER-FREE). Fixed for the DIRECT forms:
  - attempt 2 (`scopes.rs::insert_free`): free the copied `__work_N` at the B5-L3
    `__ret_N` hoist.
  - attempt 2d (`parse_block`, control.rs): promote a native-text-call tail (bare
    OR explicit `return <call>`) to a hidden `&text` caller buffer — pass-2-only
    (a hidden attr persists across passes; both-pass injection double-classifies).
  - Result: the `-> text?` **UAF is fully gone**; harness `--test issues` leakers
    **~113 → 42**; probe matrix 8/13 clean.

## What REMAINS — the next arc (composite / view-return text)

42 `--test issues` leakers remain. They are a DISTINCT family (NOT native-call
tails): **view-returns of text embedded in a local composite** — `a.v.0` (tuple
field), `d.ts[0]` (vector-of-text field), generic `vec[0]`, struct fields, match
arms, fn-refs, par delivery. Leak 1/call via `append_text` (return-delivery copy of
a composite-embedded text). Neighbourhood: `materialize_view_return`
(control.rs, exists for `Reference` views #306) + the `__ret_text_N` tuple hoist
(scopes.rs @P329) — both leave the composite SOURCE's embedded texts unfreed.

Groups: `p54_struct_parse_*`/`struct_enum_*`/`match_*`/`b*`; `p197`/`p329`/`p330`/
`p243` (tuple + generic-tuple text); `p227` (fn-ref); `p235`/`p4d` (par); `plan17`;
`p241`/`q4_json_string`/`b7`/`issue_437`/`n3`/`p189c`/`p213`.

## How to resume (the instrument is ready)

1. Rebuild the ASan loft on ARM (once): `RUSTFLAGS=-Zsanitizer=address` +
   `RUSTUP_TOOLCHAIN=nightly` + PATH-prefix the nightly bin (see memory) +
   `cargo build --release --target aarch64-apple-darwin --bin loft`; symlink
   `default/` beside it.
2. Pick a sub-slice (start with the biggest: `p54_struct_parse` or the `p197`
   tuple-field). Probe it with the harness:
   `ABIN=<asan-loft> VBIN=<stable-loft> probes/text-tail-return/run_matrix.sh`
   — asserts VALUE (vs `.golden`) + LEAK (runtime-owner frames, ex-`ir_read`) + UAF.
3. loft-codegen gate: get the proven-clean working bytecode FIRST (a manual rebind
   / view-materialise that reads 0-leak), then fix at the chokepoint, then re-run
   the matrix + full suite on BOTH backends. The VALUE oracle is load-bearing — it
   caught the 2b/2c regressions (empty returns) a leak-only check would have passed.

## The @PLN54 S4 flip (blocked on the above)

`miri.yml` `asan` job → `detect_leaks=1` becomes meaningful only once the 42 are 0.
Then add a narrow `lsan_suppressions.txt` (`leak:read_block` / `read_data_with`) for
the intentional Class-1 `ir_read` `Box::leak` (the ~16 round-trip lib tests). The S1
caveat holds: a Mac can't validate the Linux ASan leg — confirm there before landing.
