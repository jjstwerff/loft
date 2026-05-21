<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# User-Facing Deferred Work

Items that downstream developers using loft would notice if shipped
— bugs they'd hit in normal code, features they'd want to use,
performance issues they'd benchmark.  Drawn from the validation
matrices and roadmap; cross-referenced with `DEFERRED.md` (the
internal-deferred index) and `ROADMAP.md` (the planned-feature
ladder).

**Criterion for inclusion:** *would I include this in a release
note?*  If yes — it belongs here.  Internal refactors,
test-infrastructure work, and quality items invisible to
downstream developers stay in `DEFERRED.md` only.

**Lock-in mechanism:** every item below has a `Lock-in test`
column.  The test is `#[ignore]`d today and goes green
automatically when the fix lands.  Pre-release ritual:

```bash
cargo test --release -- --ignored 2>&1 | grep -E "^test (plan-?[0-9]|P[0-9]+|T[0-9])"
```

Items the grep shows as **ok** are ready to ship; un-ignore them
and add a release-note entry.  Items still **FAILED** stay
deferred for the next release.

**Closed-work hygiene** — see `plans/README.md § Companion indexes`
for the project-wide rule.  Short version: closed items are
removed entirely from the open table.  The "Closed-by-decision"
section below is the sole exception — those are permanent
"will not ship" markers (non-goals), kept as historical record so
a future contributor finds the decision before re-proposing.

---

## Open user-facing items

| Item | What user hits today | Workaround | Surfaced by | Lock-in test |
|---|---|---|---|---|
| **`hash[key] = value` doesn't insert (@P305)** | Indexed-assignment into a keyed collection updates an existing key but is a SILENT NO-OP for a new key — `h[2] = …` adds nothing when 2 isn't present.  The natural map idiom can only mutate keys that already exist | Insert with `h += [T{…}]`; for insert-or-update guard `if h[k] { … } else { h += [T{…}] }` | gridmesh dirty index (lib-plan 19) 2026-05-21 | `hash_spec.loft` (plan 44) C11/C12/C25/C26 — file with the fix |
| **`keyed += [dup-key]` doesn't dedup (@P306)** | Appending an entry whose key already exists creates a SECOND entry (`len` grows); lookup returns the first.  `hash` and `sorted` both | Guard: `if !h[k] { h += [T{ck:k,…}]; }` | gridmesh dirty index (lib-plan 19) 2026-05-21 | `hash_spec.loft` (plan 44) C09/C10 — file with the fix |
| **`s = s + s` self-double native rejection (@P222, S2)** | `s = "ab"; s = s + s;` works on interp (`"abab"`) but native rejects with E0502.  @P217's fix didn't reach this case because both RHS pieces are the destination var | Hoist into a temp: `t = s; s = s + t;` | @P217 fix-variant probing 2026-05-05 | none yet — file with the fix |
| **Mixed yield-shapes in coroutine (@P226, S2)** | `fn nums() -> iterator<integer> { yield 1; for n in [10, 20] { yield n; } yield 99; }` rejects under native with E0425 — the vector literal's `__vdb_*` backing local is declared in one state arm but referenced from another.  Same family as @P218 / @P224 (state-arm scoping) but for auto-generated locals | Hoist the vector outside the generator, or restructure to a single segment shape (all Simple yields OR one ForLoopBody) | @P219 fix-variant probing 2026-05-05 | none yet — file with the fix |
| **Server-side handshake BufReader can lose initial frames (@P221, S2)** | Latent: the server's HTTP `parse_request` uses `BufReader::new(stream)` and drops it at end of function.  If a client sends a WS frame *before* the server's `101 Switching Protocols` response (eager / non-standard clients), those bytes get absorbed by the BufReader and lost on drop.  Browser clients wait for `101` so this is invisible in practice, but the symmetric client-side bug was fixed in commit `113dc8a` (`lib/web/native/src/ws_client.rs:180`); the server side wasn't | Don't send WS frames before receiving `101` (every standards-conformant client already does this).  Fix shape on the server side: byte-by-byte header read mirroring the client fix | Code-review finding 2026-05-05 | none yet — file with the fix |
| **Vector of non-capturing closures (@P214, S2)** | `vector<fn(...) -> ...>` of non-capturing lambdas panics under interp; native rejects with E0605.  Documented-supported shape doesn't actually work | Define each fn at top level and put the names in the vector | @PLAN15 D4 (non-capturing) pre-flight | none yet — file with the fix |
| **Closure-typed var unreachable from inside another closure (@P215, S2)** | `inner = fn(x:integer) -> integer { … }; outer = fn(y:integer) -> integer { inner(y) + 1 }` rejects with "Unknown function inner" | Inline the inner body, or make both top-level fns | @PLAN15 C6 pre-flight | none yet — file with the fix |
| **Text-element tuple capture native rejection (@P228, S2)** | `t = ("hello", 42); label = t.0;` where `t` is captured emits `let mut var_label: String = &var_t.0.to_string();` (Rust method-call precedence — `&` binds outside `.to_string()`) → E0308.  Interp works | Bind to a separate variable BEFORE capturing: `t0 = t.0; label = t0;` | @P216 fix-variant probing 2026-05-05 | none yet — file with the fix |
| **Implicit generic-tuple type inference** | `t = min_max(7, 3); t.0` rejects with "Expect token ;" — the parser doesn't propagate the substituted return type to the receiving variable | Annotate the receiving slot: `t: (integer, integer) = min_max(...)` (then `t.0` works) | @PLAN17 phase 01 (A) caveat | `tests/issues.rs::plan17_a_implicit_generic_tuple_type_inference` |
| **`name @ pattern` inside or-patterns** | `match n { x @ 1 \| x @ 2 => x*10, _ => 0 }` rejects with "Expect token =>"; the parser doesn't recognise `name @ pattern` in the or-pattern loop.  No longer hangs (@PLAN18 phase 01 fix) but the syntax doesn't work | Either: (a) bind in the arm body — `1 \| 2 => { x = n; x*10 }` — or (b) split into separate arms | @PLAN18 phase 01 feature decision | `tests/parse_errors.rs::plan18_at_binding_in_or_pattern_does_not_hang` (regression-only; **add lock-in for the feature**) |
| **Tuple-returning par workers** | `for x in v par(r = make_tuple(x), 4) { ... }` rejects "Parallel worker return type 'tuple(...)' (size 16) is not supported" | Wrap result in a single-field struct (`struct Pair { v: (A, B) }`) and have the worker return `Pair`, then `r.v.0` / `r.v.1` | @PLAN06 phase 9c | `tests/threading_chars.rs::par_tuple_return_int_int`, `_int_text`, `_struct_text`, `_three_arity`, `_nested` (5 ignored canaries) |
| **Tuple destructure in fused-for-par** | `for (a, b) in pairs par(r = work(a, b), 4) { ... }` rejects "Expect variable after for" | Pre-bind: `for p in pairs par(r = work(p.0, p.1), 4) { ... }` | @PLAN06 phase 9d | `tests/threading_chars.rs::par_tuple_destructure_in_for` |
| **Capturing closures in `vector<fn(...)>`** | `vector<fn(integer) -> integer>` of heterogeneous capturing lambdas fails at vector-construction with type-inference or store-write panic.  Non-capturing lambdas and named fn-refs work fine | Restrict to non-capturing lambdas, named fn-refs, or homogeneous closure types | @PLAN06 phase 1 retrospective + @PLAN15 D4 | `tests/threading_chars.rs::par_vec_of_capturing_fns_t4` |

---

## Closed-by-decision (will not ship)

Items declined as design non-goals.  Recorded here so a future
contributor finds the decision before re-proposing.

| Item | Decision | Rationale | Reference |
|---|---|---|---|
| `iterator<T>` as par input or output | Non-goal | Generators are sequential; par needs random-access input. Worker-state-handoff has no real consumer | DESIGN.md D11c.2 (@PLAN06) |
| Tuples in struct fields (T1.11a) | Originally rejected, **lifted in 0.8.4** — tuple fields work via inline layout (synthetic `__tuple<…>` struct) | per `tests/parse_errors.rs` line 765 comment | TUPLES.md / `parser/mod.rs::set_field_check`/`get_val` Type::Tuple arms |
| Single-element tuples `(T,)` | Non-goal — `(T)` is just `T` | TUPLES.md |
| Named tuple fields `(name: T, value: U)` | Non-goal — use a named struct | TUPLES.md |
| Whole-tuple format-string interpolation `{t}` | Non-goal — access elements explicitly | TUPLES.md |
| Vector-of-capturing-closure with **heterogeneous** captured shapes | Restriction documented per loft-write skill; lift requires fundamental vector-storage rework | DESIGN.md D11a row 8 (split) |
| Dynamic dispatch / interface values (`x: Ordered = …`) | Non-goal — interfaces are constraints, not types | INTERFACES.md |
| Composite interfaces / interface inheritance | Non-goal | INTERFACES.md |
| Default method implementations on interfaces | Non-goal | INTERFACES.md |
| Associated types | Non-goal | INTERFACES.md |

---

## Severity override — when to break "finish plans first"

The default discipline is **finish the validation plans before
shipping new feature work** (see plans/README.md preamble + the
"finish-plans-first" stance recorded in this branch's session
history).  Validation work compounds; splitting attention across
plans + features is slower than serialising.

**But:** an item in the open table above can override that
discipline if its severity is high enough.  If a user-facing item
is **too aggrieving to ship a release with**, fix it before
release regardless of which plan it belongs to and regardless of
how many plan phases are still in-flight.

### Severity thresholds — `Severity` column

Each open row carries an implicit severity tier.  Make the call
explicit when uncertain.  The tiers, from must-fix-now to
defer-acceptable:

| Tier | What it means | Examples from the current table |
|---|---|---|
| **S0 — release-blocking** | User code that follows the documented language reference fails or crashes.  Visible in basic patterns; no reasonable workaround.  Example: a runtime panic on `sorted<>` cleanup that we can reproduce. | (none today; @PLAN20's panic was here but doesn't reproduce) |
| **S1 — ship-with-caveat-OK once** | Visible to users but with a clear, documented workaround that idiomatic loft code uses anyway.  Acceptable for one release with a release-note caveat; **must be fixed before next release**. | "Implicit generic-tuple type inference" (workaround: explicit annotation); "Bounded-T method-call return inference" (workaround: typed local) |
| **S2 — niche / advanced-pattern** | Affects an idiomatic shape but not the language's primary surface.  Workaround is straightforward; users can avoid the path entirely. | "`name @ pattern` inside or-patterns"; "Tuple-returning par workers"; "Capturing closures in `vector<fn(...)>`" |
| **S3 — cross-cutting future-feature** | Fix is part of a planned feature that hasn't been scheduled yet.  Documenting the gap is the deliverable. | "Closure-DbRef leak" (planned @PLAN15 phase 03); coroutines × par non-goal |

### Override decision rule

If the open table contains:
- **any S0 row** → block release until fixed.
- **an S1 row that's been deferred for two releases in a row** → promote to "must-fix-before-release" status; if it can't be fixed in time, slip the release.
- **only S2/S3 rows** → ship; document workarounds in release notes.

When promoting an S1 → must-fix:
1. Write a `Trigger to unpause:` line on the deferring plan if not already present.
2. Move the work to the *front* of the next session's queue.
3. Add the override note to the release-prep checklist.

This rule keeps the "finish plans first" discipline as the
*default* while preserving the right to override when severity
demands it.  The bar for override is "user code can't do its job"
(S0) or "we keep deferring this and it's been long enough" (S1
with two-release decay).  Pure feature gaps (S2/S3) wait their
turn in the matrix.

---

## Strategic showcase track — recruitment deliverables

Distinct from severity-driven open work above.  Items here are
**strategic deliverables** that attract developers to the project:
visible demos, performance showpieces, "look what loft can do"
moments.  They're not severity-tiered because they don't represent
broken behaviour — they represent **opportunities**.

**Priority axis:** these advance when natural breakpoints occur
between validation phases (a heavy bug-fix session closes; the
matrix's next plan hasn't opened yet; bug yield drops below
threshold).  They do **not** displace validation work — improving
loft is the first priority; recruitment work fills the gaps where
validation is between phases.

**Why a separate track.**  Severity-driven open work answers "is
this broken?"  Strategic-recruitment work answers "is this
worth showing?"  The two are orthogonal — a feature can be
unbroken-but-unshowcased (no S-tier, but a recruitment opportunity)
or broken-but-already-flashy (S-tier, irrelevant to recruitment).
Mixing them in one queue led to mis-sequencing in the May 2026
session: A10 (browser parallel) was almost promoted to "next
milestone" on recruitment value alone, but its severity is S2 and
that's the real deciding factor for sequencing against validation
plans.

### Open showcase items

| Item | Why it attracts developers | What it needs | Track status |
|---|---|---|---|
| **`brick-buster.html` — playable game in browser** | "Loft programs run real games in your browser" — concrete proof-of-life that the language works for non-trivial workloads.  Already a partly-built page at `doc/brick-buster.html`; needs the underlying infra to ship for the demo to feel responsive. | A10 (browser parallel via wasm-bindgen-rayon) for parallel chunk-mesh generation; possibly A7 if mesh workers return tuples; world/chunk app-level loft code. | OPEN — fills natural gaps in validation work; advanced when sessions have room. |
| **Parallel chunk-mesh world rendering (browser)** | Multi-threaded WASM running a 3D world in the browser is a strong demonstration that loft scales beyond toy examples.  Per ARC.md A10 sub-deliverable. | A10 8a (`wasm-bindgen-rayon` integration + COOP/COEP), 8b (rebase walk after `postMessage`), 8c (cache coherence + tests). | OPEN — same parallel-track scheduling as brick-buster. |
| **Native OpenGL world demo** | Demonstrates loft's native build is production-ready for game/sim workloads.  Builds on `lib/graphics/` (`mesh.loft`, `scene.loft`, `render.loft`) and the Moros-editor OpenGL infrastructure (already shipped per `finished/03-native-moros-editor`). | App-level loft code only — no interpreter work needed.  Reuses existing par over `vector<Reference<Chunk>>` returning Mesh. | OPEN — pure application code; can be done by any contributor with no @PLAN06 dependency. |
| **Performance showcase: parallel-WASM benchmark suite** | Concrete numbers showing browser-parallel vs sequential vs native.  Same shape as `bench/11_par/`, adds a browser column. | Depends on A10 landing.  Adds `bench/12_browser_par/`. | OPEN — natural follow-on to A10 8c. |

### How showcase items move through this section

```
OPEN (strategic) ──A10/8a/8b/8c lands──► IN-FLIGHT (demo polish)
                                                │
                                                ▼
                                        SHIPPED (release notes)
                                                │
                                                ▼
                              row removed; demo is now a permanent
                              gallery entry / blog post / readme
                              feature
```

Each shipped showcase item gets a release-note line AND a project-
level visibility update (`README.md`, `gallery.html`, project
landing page).  These are the artifacts that bring developers in.

### Sequencing rule

Severity-driven open work is the **primary track** — every session's
default candidate.  Showcase items are the **secondary track**
worked when:
- the session has time after the primary work is in a stable state, OR
- a primary plan reaches a natural breakpoint (phase closes,
  pre-flight survey shows the next phase is low-yield), OR
- a showcase item gates an external commitment (specific demo
  date, conference talk, contributor's reasonable expectation).

The default ordering when starting a session: pick from the
severity table first.  Only if no S0/S1 work is open AND no S2 work
is mid-investigation, advance the showcase track.

### Today's snapshot (2026-05-04)

**Severity-driven open work** — all 8 rows are S1 or S2:
- "Implicit generic-tuple inference" — S1 (would block 1.0 if
  still unfixed by then; documented workaround exists for now).
- "Bounded-T method-call return inference" — S1 (same shape).
- "`name @ pattern` in or-pattern" — S2 (alternate syntax exists).
- "Native char-tuple-elem comparison" — S2 (cast workaround).
- "Tuple-returning par workers" — S2 (struct-wrap workaround).
- "Tuple destructure in fused-for-par" — S2 (pre-bind workaround).
- "Capturing closures in `vector<fn(...)>`" — S2 (named-fn-ref
  workaround).
- "Closure-DbRef leak" — S3 (no current symptom; long-running
  programs only).

**No S0 today.**  Plan-20's panic would have been S0 if it
reproduced; it doesn't, so it's deferred with a re-surface
trigger.  Continue with the "finish plans first" default.

**Strategic showcase track** — 4 open rows (none broken; all are
visibility opportunities).  Today's work pulls from the severity
table first; showcase items advance when validation work hits a
natural break.  No S0 escalation has consumed validation time;
similarly no showcase item is yet at the "external commitment"
gate that would justify pulling it forward.

---

## Cross-references

- [DEFERRED.md](plans/DEFERRED.md) — internal-deferred work index
  (every parked item with its trigger to resume).
- [ROADMAP.md](ROADMAP.md) — planned-feature ladder by milestone
  (0.9.0 / 1.0.0 / 1.1+).
- [PROBLEMS.md](PROBLEMS.md) — open P-issues; closed entries removed
  per file convention.
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — long-form rationale
  for closed-by-decision items above.
- [CHANGELOG.md](../../CHANGELOG.md) — user-facing release notes.
