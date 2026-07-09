<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN53 — verifiable small steps for the rest of the plan

Companion to [`README.md`](README.md) (the arc catalogue) and
[`F1-DESIGN.md`](F1-DESIGN.md) (F1's design gate). This decomposes the **open**
work — the F1 residual, F2, F3, F5 — into small steps, each with a **local
verification that runs on this box**. F1.0–F1.5 already landed; they are the
template these follow.

## The verifiability principle (why these steps are checkable without a fuzzer)

This box has **no nightly and no cargo-fuzz**, so `cargo +nightly fuzz run`
cannot run here. F1 dodged that by a structural move, and every arc below reuses
it:

> **Reify the fuzzer's logic as a library function; make `cargo test` the
> correctness gate; keep the `fuzz_target!` a two-line shim over it.** A
> generator returns a `String`; an oracle returns a clean/finding verdict. Then
> a `#[cfg(test)]` test drives the *same* function over many inputs on stable —
> that test IS the exit criterion, and the libfuzzer target only adds
> coverage-guided mutation on top.

So each arc's steps are ordered **most-value-verifiable-here first**; the
genuinely env-gated steps (an actual fuzz run, the wasm leg, OSS-Fuzz Docker)
come last and are marked **[env-gated]**. A step is "done" only when its local
gate is green on **both backends** where it runs code (the both-backends rule).

Two arcs open with a **design gate** (`*.0`, like F1.0) because their invariant
is not yet formable from the desk — build the cheapest instrument that *plots
the answer* (a hand-written exemplar program) and read the grammar/rule off it,
per the design-protocol skill.

---

## F1 — residual (design complete; F1-DESIGN.md)

| Step | Produces | Local verification | Exit |
|---|---|---|---|
| F1.4-run **[env-gated]** | An actual coverage-guided run | `cargo install cargo-fuzz` + nightly, `./fuzz/seed_program_source.sh`, `cargo +nightly fuzz run program_source -- -runs=1000000` | run stays up; any crash artifact triaged |
| F1.5-triage (ongoing) | Each new finding fixed-or-recorded + a regression case | `cargo test --lib fuzz_oracle::tests::seed_corpus_replay -- --ignored` green after each | F1-1 done; new findings closed the same way |

F1 needs no more design. The only thing it cannot do here is *run the fuzzer*.

---

## F2 — keyed-container axis (`hash`/`sorted`/`index` + closures)

The plan's #1 resume trigger, and the hard one: a generated `hash`/`sorted`
program is **schema-coupled** (`Key{type_nr,position}` indexes the type
registry; RB-node layout interleaves user fields, the key, and links), so a
valid-by-construction grammar is not obvious. Hence a design gate first.

| Step | Produces | Local verification | Exit |
|---|---|---|---|
| **F2.0** design gate | `F2-DESIGN.md`: the ONE invariant a generated keyed-collection program self-checks (e.g. *after N inserts + D deletes, `len == N−D`, every surviving key looks up its inserted value, iteration visits exactly the survivors*), + the grammar read off **hand-written exemplar programs** | Hand-write 2–3 exemplar `hash`/`sorted`/`index` programs; run on `--interpret` **and** `--native`; confirm each is valid + its self-check passes | invariant + grammar pinned; exemplars run clean both backends |
| **F2.1** reify generator | `generate_keyed(spec) -> String` in a lib module (behind `fuzzing`+`cfg(test)`, like `fuzz_oracle`) emitting valid-by-construction keyed programs | `cargo test`: generate K programs across the spec space, run each in-process (parse→compile→execute); assert **all** compile + run clean (a rejected program is a GENERATOR bug, not a finding) | generator emits only compiling programs; `hash`/`sorted`/`index` ops covered |
| **F2.2** self-check + falsify | Each generated program asserts the F2.0 invariant | `cargo test`: (a) all self-checks pass; (b) a deliberately mis-generated program (planted broken invariant) **fails** — proves the harness can fail before trusting it | planted violation is caught |
| **F2.3** closure/lifetime axis | Nested closures + many overlapping variable lifetimes woven in (stresses the slot allocator) | `cargo test` over the extended grammar; all compile + run + self-check | closure programs compile and run |
| **F2.4** poison sweep | Run the generated corpus under arena poison-on-free (F4) | `cargo test` in-process with poison on (opt-in env, like `LOFT_F1_POISON`); a keyed-collection UAF becomes a loud finding | corpus clean under poison, or findings triaged |
| **F2.5** libfuzzer target **[env-gated run]** | `fuzz/fuzz_targets/program_keyed.rs` — a two-line shim over `generate_keyed` + run | `cargo check --bin program_keyed` (stable); actual `cargo +nightly fuzz run` env-gated | target type-checks; run documented |
| **F2.6** triage loop | findings fixed-or-recorded + regression cases | `cargo test` after each | new findings closed |

Note: this reifies the generator the way F1 did, which `program_ownership`
(the existing F2 partial) does **not** — so F2.1's pattern can be back-applied
to give `program_ownership` a stable `cargo test` gate too.

---

## F3 — differential interp ≡ native ≡ wasm on *fuzzed* programs

The pieces exist: F2's generator + F1's `classify_source` + @PLN89's 3-backend
differential oracle (today over a fixed ~29-program corpus). The open design
question is **noise**: a generated program can diverge *legitimately* (unordered
iteration in output, timing). So a design gate settles what "comparable" means
before wiring.

| Step | Produces | Local verification | Exit |
|---|---|---|---|
| **F3.0** design gate | `F3-DESIGN.md`: the **deterministic-output subset** rule — the generator constraints (or output canonicalization) under which *any* interp/native/wasm divergence IS a finding, not noise | Take ~5 generated programs; run 3 backends; confirm the constrained subset agrees and catalogue the divergence sources the rule must exclude | the comparable-output rule is pinned |
| **F3.1** differential test | A `cargo test` that generates programs (F2 generator, restricted to the F3.0 subset), runs each on the backends, compares output+exit | `cargo test`: **interp ≡ native runs HERE**; the wasm leg reuses @PLN89's existing gating (nightly/wasm runner) | differential green over the generated corpus; divergences flagged |
| **F3.2** triage | Each divergence → matrix → fix-or-record (a real cross-backend codegen bug) | `cargo test` per case | divergences closed |

F3 is smaller than F2 — it composes two existing subsystems; the only new
design is the F3.0 determinism rule.

---

## F5 — OSS-Fuzz onboarding **[env-gated + precondition-gated]**

Lowest readiness: no design, and it must not start until the targets are
crash-clean (a crashy continuous run is noise, not signal).

| Step | Produces | Local verification | Exit |
|---|---|---|---|
| **F5.0** precondition gate | Confirmation that `program_source` (+ `program_ownership`, `program_keyed`) run crash-clean for a sustained window | The in-process replays/sweeps green + a long F1.4/F2.5 run with no unfixed crash | no open crashers |
| **F5.1** project skeleton **[env-gated]** | `oss-fuzz/` `project.yaml` + `Dockerfile` + `build.sh` (builds the targets in the OSS-Fuzz image) | `python infra/helper.py build_fuzzers loft` in the oss-fuzz clone (needs Docker) | targets build in the OSS-Fuzz image |
| **F5.2** seed + dictionary **[env-gated]** | Packaged seed corpus (reuse `seed_program_source.sh`) + a loft-keyword `.dict` | `helper.py check_build loft`; a short `run_fuzzer` smoke | fuzzers run in-image |
| **F5.3** submit **[external]** | PR to `google/oss-fuzz` | maintainer review | loft accepted; targets running |

---

## Suggested order

1. **F2.0 design gate** — highest value, the plan's #1 resume trigger, and the
   only place a `design-protocol` pass is load-bearing (schema-coupled grammar).
2. **F2.1–F2.4** — the stable, verifiable-here build (generator + self-check +
   poison sweep). This is where F2 becomes real without a fuzzer.
3. **F3.0 + F3.1** — smaller; compose F2's generator with @PLN89's oracle once
   F2 emits programs.
4. **F1.4-run** whenever nightly+cargo-fuzz are installed (independent of the
   above; just needs the tooling).
5. **F5** last, only after F1.4/F2.5 runs are crash-clean.
