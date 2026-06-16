<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# lib-test fixed-filename concurrency (surfaced during Phase C)

A `find_problems` run during the rc-removal Phase C work showed
`lib/moros_render/tests/geometry.loft::test_map_export_glb_creates_file` FAILED
("GLB file written" assert).  Rigorous probing showed this is **NOT a Phase C /
file-write bug** — it is a pre-existing **test-concurrency** artifact.  Phase C is
innocent (it only shifted timing).

## The mechanism

`geometry.loft` writes a **fixed, cwd-relative** filename and then `delete()`s it:

```
xpath = "moros_render_test.glb";   // @P333: cwd-relative (Windows has no /tmp/)
map_export_glb(xm, xpath);
assert(file(xpath).exists(), ...);
delete(xpath);
```

The lib-test harness runs each test with `current_dir(pkg_dir)` (`tests/wrap.rs:439`).
The **interpreter** `library_suite` (wrap) and the **native** `native_library_suite`
(native) are separate test binaries that BOTH run `geometry.loft` with the same
CWD = `lib/moros_render/`, and `find_problems` (cargo-nextest) runs them
concurrently.  So two processes write+assert+`delete` the **same file** in the
**same directory**: one process's `delete` removes the file the other just wrote,
before its `exists()` assert → flaky FAIL.

## Decisive evidence

| run | result |
|---|---|
| `loft test geometry` **standalone** (1 process) | **105/105 pass** (GLB written) |
| two `loft test geometry` **concurrent**, same dir | **flaky** — round 1 proc-B fails, round 2 proc-A fails, round 3 both pass |

Plus the file-write/close MECHANISM is correct under Phase C (the `.loft` probes
here — aliasing, pass-by-value, callee-created, multi-write, loop-churn — all
pass on BOTH backends).  Phase C's file-close change (close on every File-store
free, was gated on `ref_count <= 1`) does not break file writing.

## Root cause + fix — FIXED (harness isolation)

The root is **two concurrent processes + a fixed cwd-relative filename + no
per-process uniqueness**.  loft has no `random`/`pid`/`tempfile` primitive, so a
test can't make the name unique itself.  Affected: `moros_render/geometry.loft`
(3 names), `moros_sim/persistence.loft`.

**Fix (option 1 — harness isolation, `run_lib_test_in_temp_cwd` in `tests/wrap.rs`
+ `tests/native.rs`):** each lib test now runs in a UNIQUE `.loft_test_tmp_<pid>_<n>`
SIBLING dir inside `lib/`, with the package's contents symlinked in.  Being a
sibling inside `lib/`, the package's relative deps (`../<name>`) still resolve to
the real packages; cwd-relative artifacts land in the unique dir (removed after).
Discovery (`collect_library_tests` + the native loop) skips dot-dirs so the temp
dirs are never picked up as packages.  Verified: geometry passes 105/105 isolated;
concurrent isolated runs no longer race; interp `library_suite` + native
`native_library_suite` both green through the new path; no leftover temp dirs.

(Considered + rejected: nextest-serialize the two suites — loses parallelism; add
a loft uniqueness primitive — touches the language + every fragile test.)

This was **off the rc-removal path** — a pre-existing test-infra bug surfaced by
the Phase C run, fixed in the same branch.
