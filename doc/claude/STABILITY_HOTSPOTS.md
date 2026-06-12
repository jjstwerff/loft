<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_HOTSPOTS.md — the structures that will keep manufacturing bugs

> Open H-items are ORDERED in [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md) —
> the single tracking view across all stability docs.  This file stays the
> canonical home for each hotspot's invariant, evidence, and mitigation design.

Companion to [STABILITY_METHOD.md](STABILITY_METHOD.md) (the three-pass
method) and [STABILITY_PASS2.md](STABILITY_PASS2.md) (the executed
relocation pass).  Where those documents look at *routines*, this one looks
at *designs*: the places where the 2026-06 bug history (#299, #306, #313,
#328, #330, #336, #339, @P364, @P377, plan-57's tail) clusters so tightly
that the next dozen bugs are predictable.  Each hotspot names the violated
invariant, the evidence, and the mitigation work — sized, ordered, with its
validation gate.

Assessment date: 2026-06-11 (branch `bugs325`; 5 open issues, all
`fixed-pending-merge`).  The codebase is in its best state so far — this
document is about keeping it that way as features land.

## Reading the sizes

`S` = under a day · `M` = days · `L` = a plan with phases (design doc +
quiet window).  Every L item must run through
[DESIGN_PROTOCOL](DESIGN_PROTOCOL.md) (name the invariant, count
re-assertion sites, falsify the load-bearing claims) before code.

## Landing order (recommendation)

```
H1 (signature ABI)  ──── unlocks ────▶  H5 (deletes most of the contract)
H2 (typed deps)     independent, do early — converts corruption to compile errors
H6 (sentinel codec) independent, S-sized — do in any gap
H7 (codec derivation / F9)  before the next Value/Type variant lands
H3 (ownership as data)  AFTER H1+H2 (both shrink its surface)
H8 (allocations privacy)  the already-planned privacy pass
H4 (backend unification)  continuous discipline now; structural fix is 1.1+ scale
```

---

## H1 — Analysis-dependent function arity (the #1 future bug factory)

**The violated invariant**: *a function's signature should be a pure
function of its declaration.*  Today `ref_return`
(`src/parser/control.rs`) GROWS a fn's attribute list during parsing,
based on NRVO analysis of its body — which depends on its callees'
(also growing) signatures.  That is a call-graph fixpoint being
approximated by "two passes + cross-pass name stability + a retro-patch".

**Evidence**: #299 (wrapper returns aliased buffer), #306
(attr-index/var-number dep confusion), @P364/@P377 (collapse hazards over
the hidden-buffer IR), #339 (late pass-2 growth breaks earlier callers).
#339's fix history is the strongest signal: two simpler fixes failed in
opposite directions — freezing signatures after pass 1 silently
re-introduced #299's corruption (promotion is load-bearing), and a
conditional third parse pass segfaulted `100-enhancements` (per-fn
variable tables are name-stable by design, not re-entrant).  When one bug
has three competing wrong fixes, the design is the bug.

**Mitigation — unconditional heap-return ABI (L)**

Make the hidden return-buffer parameter a property of the RETURN TYPE, not
of body analysis: every fn returning `Reference` / `Vector` /
struct-`Enum` gets the hidden buffer attr at declaration-parse time,
always.

1. Design doc (plans/ slot): enumerate every consumer of "does this fn
   have a hidden buffer?" — `add_defaults`, `collect_hidden_ref_args`,
   `nrvo_collapse_tail_set`, `filter_hidden`, codegen's `OpReturn` width
   logic, the cdylib export wrappers (`native_lib.rs` marshals by
   attribute list), fn-ref dispatch, `introspect`.  Count the
   re-assertion sites per DESIGN_PROTOCOL.
2. Add the attr unconditionally in `parse_fn_signature` (pass 1, before
   any body parses).  `ref_return` shrinks to: decide what flows INTO the
   already-existing buffer (NRVO yes → callee builds in place; NRVO no →
   the copy path fills it).  No `add_attribute` outside signature parse.
3. Delete the machinery the change strands: `retrofit_callers_hidden_args`
   (#339's fix becomes dead), the `__rref_` recursive-self dance in
   `add_defaults` (arity can no longer differ between passes), the
   `signatures-must-not-grow` comment contracts.
4. Cost check: fns that today DON'T promote (reassigned locals, literal
   tails) would now carry an unused buffer param → one extra `DbRef` arg
   per call.  Probe the perf delta on the benchmark suite
   ([PERFORMANCE.md](PERFORMANCE.md)); if measurable, the buffer can be
   passed as the null sentinel and lazily allocated — the ABI stays
   uniform either way.
5. Validation: full matrix on both backends + wasm; the #299/#339/#306
   regression files; crawler + moros + brick-buster end-to-end; cdylib
   dispatch tests (`native_library_suite`) — the C ABI of exported
   wrappers changes shape, so `make rebuild-native-cdylibs` artifacts and
   the registry's `verified` libraries need a re-verify pass.

**Status (2026-06-11, end of day): PHASE 1 SHIPPED** — every
body-carrying plain fn returning Reference / Vector / struct-Enum
carries its hidden `__retbuf` from signature parse; `ref_return` binds
promoted locals by role swap; arity can no longer grow behind a parsed
caller (debug-asserted; lambdas keep in-place growth — no earlier
callers can exist).  The dispatcher census that gated it is complete:
plain calls, par lanes (+ runtime witness-free), entry invocations
(REPL capture), and the cdylib shared bridge (runtime type-name ids)
all speak the uniform ABI.  Full matrix green —
[plans/59-return-abi](plans/59-return-abi/README.md) records the three
rounds, every probe, and the phase-2 cleanups.  Originally: plan opened —
[plans/59-return-abi](plans/59-return-abi/README.md) (@PLAN59).  Phase 0
SHIPPED: the H1 census probe caught a LIVE #339 sibling (vector-literal
tails promote late too; 7-line caller-first repro panicked on main) — the
retrofit now covers all three heap-buffer kinds, regression in
`tests/scripts/295`.  Census results validate the one-buffer design
(ls ≤ 1 in 104/104 promotions); claims C1–C5 probed/read, C6–C8 (argument
order ↔ attr order coupling, buffer-vs-return-value consumption,
null-init suppression) are the named pre-implementation probes for
phase 1's own window.

**Until then** (standing rule, loft-write skill): a thin wrapper around a
struct-returning fn is safe ONLY if the wide impl is defined before it in
the file; the widened retro-patch covers the rest but new shapes (fn-refs
to late-promoted fns, generics) should be probed before relying on them.

---

## H2 — The `Vec<u16>` dep-list overload

**The violated invariant**: *one type, one meaning.*  The dep vector on
`Type::Reference/Vector/...` means, depending on context: caller variable
numbers, callee ATTRIBUTE indices, the `u16::MAX` pointer-share marker
(#328), a self-dep-as-ownership-marker (@P302 keyed locals), or
"empty = owned".  Readers must know which meaning the writer intended;
nothing checks it.

**Evidence**: #306 (attr indices read as var numbers → caller store freed
under a borrow), #328 (`u16::MAX` marker invented because nothing else
could express pointer-ness), #330 (a self-dep silently flipped codegen to
`InitCreateStack` — the SIGSEGV took a session to trace), `add_defaults`'
"#306: strip the callee-internal dep list" comments at three sites (each
a manual re-assertion of the convention).

**Mitigation — typed dep semantics (M)**

Step 1 DONE (2026-06-11): **[DEPS_INVENTORY.md](DEPS_INVENTORY.md)** —
the semantic model (two address spaces + five marker overloads, incl. TWO
different `u16::MAX` meanings), all 84+ reader/writer/converter sites
classified, the crossing sites named, and a corpus probe showing the
out-of-range guards in `is_borrowed_view` / `dep_has_var` are fossil
defenses (zero contamination in stdlib + moros + crawler + scripts).
Steps 2–4 DONE (2026-06-11): `Deps` is a constructor-checked NEWTYPE —
every creation site states its meaning (`none` / `frame` / `attrs` /
`pointer_marker` / `share_sentinel` / `unknown`), reads go through
`Deref` or the space-asserting accessors (`frame_vars` /
`as_attr_indices`, debug-tag-checked, zero release cost),
`resolve_deps`/`ref_return` are typed as THE converters, and contaminated
reads scream in debug.  The step-3 bisect upgraded the inventory: the
`dep_has_var` "fossil" was LIVE — block-result deps were mixed-space by
contract (in-range = attr index, out-of-range = frame var; removing the
arm made `26-closures`' factory results share one record).
Step 5 DONE (2026-06-12): the positional contract is RETIRED — the
callee-frame note a closure factory stores in `def.returned` is an
in-band TAGGED value (`Deps::CALLEE_FRAME_BIT`; sole writer
`Deps::callee_frame1` at the vectors.rs lambda propagation; decoded by
`Deps::entries`), chosen over the debug-only tag because VALUES survive
the IR codec (cache round-trips erase the debug tag — corpus-probed).
The block-result dep read in `get_free_vars` is deleted (probed across
five corpora: never decides alone; a debug sentinel guards the claim);
`check_ref_leaks` pools the decoded note instead of dropping it (its
false `___clos_1` leak report is gone).  Regression:
`tests/scripts/297-closure-factory-explicit-return.loft`.  Full record +
residuals found en route (armed-lib-debug baseline redness; @PLAN59
growth assert on two lib fns): DEPS_INVENTORY § Status.

---

## H3 — Ownership recomputed by shape analysis

**The violated invariant**: *a fact should be stored where it is decided,
not re-derived where it is needed.*  `src/scopes.rs` (4,668 lines)
re-derives ownership, escape, confinement, and free-placement facts by
pattern-matching IR shapes — every analysis re-asserts what construction
already knew.  Every new construct must be taught to each analysis or it
silently misclassifies.

**Evidence**: plan-57's entire tail (probe-numbered exclusions, the gated
`LOFT_CONF_RECOVER` experiment, cluster I/III routes), #260 (the
`Set(v, Null)` position doubling as the declaration point — fixed this
cycle by moving the fact INTO the variable table, the model for this
hotspot), #316/#323 (five ownership homes), the dominance-twin drift
found in pass 3.

**Mitigation — ownership as carried data (L, after H1+H2)**

1. The variable table (`src/variables/`) becomes the single home for
   per-var ownership state: owned / borrowed-view / caller-buffer /
   captured — written ONCE at the construction site, read by every
   scopes analysis instead of re-derived.  (#260 Fix B and
   `mark_caller_hidden_buf` are the first two facts already living
   there; continue the pattern.)
2. Convert one analysis at a time, matrix-first: `guard_escapes` →
   `reclaim_safe` → confinement.  Each conversion keeps the old shape
   walk as a debug-assert cross-check for one release
   (`debug_assert_eq!(carried, derived)`) — drift between them is a
   found bug, exactly like the pass-3 unifications.
3. The walker keystones (`Value::any_node` family) stay as the mechanism
   for the residual genuinely-shape-local questions (tail position,
   dominance).
4. Validation: the store-leak gate, ASan run, plan-57's probe corpus,
   watermark guard (`reclaim_unfreed_eligible`).

---

## H4 — Two-and-a-half backends

**The violated invariant**: *one semantics, one implementation.*  The
fill.rs generation covers OPCODES (one `#rust` template per op — this
part is healthy).  Above the opcode level, lowering decisions are
implemented independently three times: bytecode (`state/codegen.rs`),
native (`src/generation/`), wasm (native + bridge variations).

**Evidence this cycle alone**: #260 needed PAIRED fixes (native prologue
+ interp `OpInitRefSentinel`); the x=x fix had to be hoisted to the
parser to cover both; #333's fail-fast was native-only plumbing; the
pre_eval free-op recognizers existed only because native re-walks what
bytecode also walks.

**Mitigation — discipline now, structure later**

- (standing, S per change) Any lowering-semantics change MUST land with a
  `cross_mode!` cell or a `tests/scripts/` file that both sweeps run —
  this is mostly already culture; make it a CODE.md checklist line.
- (M) Extend the `#rust`-template idea upward where shapes allow: the
  free-op family, null-init emission, and the GET→SET table are
  per-backend tables that could derive from one declaration each, the
  same way fill.rs derives ops.
- (L, 1.1+ scale) The real fix is one shared lowering IR below today's
  Value-IR (decisions made once, interpreted twice).  Do NOT attempt
  before H1/H3 — they shrink exactly the semantics that would need
  porting.

---

## H5 — The two-pass name-stability contract

**The violated invariant** (implicit, undocumented until now): *pass 2
must reproduce pass 1's synthesized names exactly* — work-ref counters,
lambda naming, attr re-finding by name all depend on it, and the parser
is not re-entrant beyond two passes (the #339 third-pass experiment
segfaulted on half-migrated variable tables).

**Mitigation (S now, mostly dissolved by H1)**

1. (S) Document the contract at the `first_pass` flag declaration
   (`parser/mod.rs`) — what must be deterministic, what is re-found by
   name, why a third pass is unsound.  Done as part of this document's
   landing if nowhere better.
2. (S) `debug_assert` the contract where it's cheap: attribute COUNT per
   def equal at end of both passes (post-H1 this becomes an invariant
   rather than an aspiration); work-ref counter equality per fn.
3. H1 removes the only known source of cross-pass signature divergence;
   after it lands, re-evaluate whether anything still relies on name
   stability beyond lambdas.

---

## H6 — The null-sentinel matrix

**The violated invariant**: *one fact (this value means null) per
type, expressed once.*  Today: `i64::MIN` (integer), `i32::MIN` (i32
fields), 255 (byte/boolean/enum), raw-0 `+1` encoding (Short),
`store_nr == u16::MAX` (DbRef sentinel) vs `(0,0)` zero-default, with
symmetric read/write conventions per width re-implemented at each
`OpGet*/OpSet*` pair.

**Evidence**: the u8/u16 nullable-width work this cycle (one code
reserved, min/max-expressed, symmetric handling had drifted), the
`byte_vec` gate subtlety in `get_val`, `default_native_value`'s
per-type arms (Spacial was missing until last week).

**Mitigation — one sentinel table (S–M)**

1. A `sentinel(tp) -> Encoding` table in `src/data.rs` next to
   `byte_width` — the ONE place mapping type×width to its null code and
   its read/write transform.  `fill.rs` templates, `default_native_value`,
   `emit_typed_null`, and the narrow-vector paths consume it.
2. A doc table in [INTERMEDIATE.md](INTERMEDIATE.md) generated or checked
   against it (the api-lint pattern), so the convention is greppable.
3. Validation: the narrow-width script corpus (p184 family, 292
   three-state boolean), both backends.

---

## H7 — Hand-maintained IR codecs (F9)

**The violated invariant**: *the codec must cover exactly the schema.*
`ir_store.rs` / `ir_schema.rs` / `ir_read.rs` hand-encode every
`Value`/`Type` variant field-by-field.  A new variant compiles cleanly
with a silent codec gap; the startup cache and G2 store-backed emission
then read garbage.

**Mitigation — derive from one schema declaration (M, design exists as F9)**

1. The `for_each_child` keystone can't drive codecs (they encode FIELDS,
   not just child edges) — F9's "one schema declaration" is the fix:
   a macro or table from which encoder, decoder, AND the exhaustive
   walker match all derive.  A new variant then breaks the build until
   all three know it.
2. Until then (S, do immediately with the next variant): a round-trip
   property test — materialize every construct in `tests/scripts/` into
   a store, read it back, assert IR equality.  Cheap, catches gaps the
   day they're introduced.
3. Validation: `arc_e_program_cache` suite, `LOFT_CODEGEN_STORE=1` run of
   the native sweep (the G2 path).

---

## H8 — The raw `Stores.allocations` surface

**The violated invariant**: *invariant-rich state needs an interface.*
60+ direct `stores.allocations[...]` touches across `parallel.rs`,
`extensions.rs`, `native.rs`, `wasm_gl.rs` — the parallel worker-slot
swap dance is the riskiest patch (store isolation during `par` is
load-bearing for memory safety).

**Mitigation — the already-planned privacy pass (M–L)**

Tracked in [STABILITY_PASS2.md](STABILITY_PASS2.md)'s deferred rows: design
the accessor surface (worker-slot claim/release, swap-back, lock APIs) as
ONE batch with [THREADING.md](THREADING.md) in hand, then convert callers
mechanically.  Not started; should precede any new `par` feature work.

---

## What this register is NOT

- Not a bug list — open bugs live in GitHub Issues; this is where the
  NEXT bugs will come from.
- Not a license to rewrite — every H-item follows the pass-2 rules:
  quiet window, matrix-first, one move at a time, suite-green between
  moves, and the old path kept as a cross-check where feasible.
- Not static — when an H-item's mitigation lands, move its entry to a
  short "retired" section below with the closing commit, the same way
  PROBLEMS.md archives fixed bugs.

## Retired

### H1 — Analysis-dependent function arity (RETIRED 2026-06-11)

Phase 1 (the unconditional `__retbuf` ABI + the dispatcher census:
plain calls, par lanes with witness-free, entry invocations, the
cdylib shared bridge with runtime type-name ids) and phase 2 (deleted:
`retrofit_callers_hidden_args`, the `grew_in_pass2` plumbing, the
`__rref_` recursive-self counter dance; documented: the two-pass
contract at the `first_pass` flag, the calling convention in
COMPILER.md § Function calling convention) — all landed on `bugs325`.
Arity is a pure function of the declaration AFTER pass 1: armed builds
showed the original "never grows" assert firing on the
multi-return-site shape (a fn whose return sites are materialized work
refs — forward-referenced callee / generated `.parse` — finds the one
`__retbuf` consumed by its first site and grows a hidden attr per
later site; seen on `moros_map::map_from_json` and graphics'
`glb_pos_min`).  PASS-1 growth there is sound — pass 2 re-parses every
caller against the final arity and re-finds the grown attrs by name,
so arity is pass-stable — and the assert now guards exactly the
dangerous clause: pass-2 growth on a plain fn (2026-06-12).  An
opt-out (leaving site 2+ un-promoted) was tried and REVERTED: the
cdylib emission of un-promoted materialized returns dereferences the
null-sentinel buffer (`map_export_glb` chain, store 65535) — recorded
in the ref_return comment.  Regression:
`tests/scripts/298-multi-return-site-ref-buffer.loft`.  H5's load
mostly dissolved with H1 (the contract is now documented + the assert
enforces its hardest clause).  Full history:
[plans/59-return-abi](plans/59-return-abi/README.md).
