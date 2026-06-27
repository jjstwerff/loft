<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Pre-work: a reusable USE-analysis phase (one producer, many consumers)

> **Status:** design (doc-first, `design-protocol`). The **foundation** the
> copy-vs-borrow elision ([copy-elision-design.md](copy-elision-design.md)) builds on,
> scoped so it is reusable by every site that today re-derives the same ownership/use
> facts from shape. This is a concrete step toward the
> [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) north star ("`deps` becomes a sound
> ownership system from which every store-lifetime codegen decision derives
> mechanically") and directly attacks the re-derivation class
> ([STABILITY_REDFLAGS.md](../../STABILITY_REDFLAGS.md), Cluster C).

---

## 1. Why a pre-work phase (the problem the optimization can't solve without it)

The materialization predicate needs **whole-function** facts about a binding `v`: is it
mutated? does it escape? does its source outlive it? But the copy-vs-borrow decision is
made **today inside `parse_assign_op`** — *mid-parse, before `v`'s later statements
exist*. So the decision can only see the **RHS shape**, which is exactly why it
mis-classifies (the #415 copy, p379, #465). You cannot key on use from a site that
hasn't seen the uses yet.

The pre-work removes that constraint: **compute the use facts once, over the fully-parsed
IR, and store them where every decision site can query them.** Without it, each tier of
the optimization (and #465, and p379, and the "& never modified" diagnostic) would
re-walk the body and re-derive the same facts — the precise re-derivation smell the
stability work exists to retire.

---

## 2. The one invariant

> **Every variable gets a `UseProfile` from a single full-IR walk; an unrecognized or
> unanalyzable use conservatively sets `mutated = escapes = true`. A borrow is therefore
> emitted ONLY when the analysis *positively proved* read-only ∧ non-escaping ∧
> source-dominating.** Soundness is by **conservative default** (unknown ⇒ the
> copy-safe classification); completeness is widened tier by tier without ever risking a
> wrong borrow.

This is the load-bearing safety property: **the analysis's "I don't know" is the safe
answer.** A missed use-shape costs a missed optimization (still copies, just slow), never
a wrong borrow (a UAF). That asymmetry is what makes the phase safe to grow
incrementally.

---

## 3. What it computes — `UseProfile` (scoped to D1/D2/D3, extensible)

Per local/binding var, keyed by `var_nr` in the function's variable table:

```rust
struct UseProfile {
    mutated: bool,          // ¬D1: any write THROUGH v —
                            //   v[i]=…, v.f=…, v += …, v reassigned,
                            //   v passed to a `&`/mutating param
    escapes: bool,          // ¬D3 (escape): v returned, stored into longer-lived heap
                            //   (x.f = v, x += v, keyed insert), captured by a closure,
                            //   or handed to a par worker
    source: Option<u16>,    // for a borrow-shaped binding `v = src` / `v = src.f`,
                            //   the source var (the borrow candidate)
    // Tier 2 (added when that tier lands; absent ⇒ treated as "can't prove ¬D2"):
    // source_mutated_in_live_range: bool
}
```

Each field maps 1:1 to a divergence event in the elision design's §2 envelope
(D1/D2/D3). The struct is **minimal for Tier 0** (`mutated`, `escapes`, `source`) and
grows only as a tier needs a new fact — the over-reach guard: do not pre-build the
whole ownership lattice for a fact no consumer reads yet.

---

## 4. Where it runs — extend `scopes::check`, not a new pass

`scopes::check(data)` (`src/scopes.rs`) already: walks each function's full `code` IR;
computes `deps` (owns vs borrows — the §3 `source`/lifetime substrate); and runs last-use
liveness (`lastuse_reclaim`). The use-analysis is the **same walk, more facts** — it
shares the traversal rather than adding a second one.

Pipeline (unchanged order): `parse` → **`scopes::check` (now also fills `UseProfile`)** →
`byte_code`. The profiles live on the per-function variable table, so they are available
to both the scopes-time rewrite (Tier 0, §6) and any later codegen consult.

**Parse stays conservative — it is not touched in the pre-work.** `parse_assign_op`
keeps emitting the **safe COPY**. The analysis phase only ever performs an *elision*
(copy → borrow) when it has *proven* safety. So:
- a bug in the analysis that under-recognizes a use ⇒ no elision ⇒ correct-but-slow;
- the analysis can never *introduce* a copy that wasn't already there.

This one-directional design (default = copy, analysis subtracts copies on proof) is what
keeps the pre-work low-risk while the consumers migrate.

---

## 5. The query API (what consumers call — the single producer's surface)

On the variable table / a `&UseProfiles` handed to consumers:

```rust
fn is_owner(&self, v: u16) -> bool;        // mutated || escapes  (⇒ must COPY)
fn borrow_source(&self, v: u16) -> Option<u16>;  // Some(src) iff v = src(.f) and !is_owner
fn proves_borrow(&self, v: u16) -> bool;   // !is_owner && source_dominates(v)  (Tier-gated)
```

A consumer never re-walks the body; it asks `proves_borrow(v)` and either borrows (on
true) or keeps the copy (on false). That is the whole point: **one producer, many
consumers, zero re-derivation.**

---

## 6. First consumer — the Tier-0 elision rewrite (proves the phase pays off)

In `scopes::check`, after profiles are filled, a peephole rewrite over the IR:
**recognize the copy-materialize idiom** emitted by `parse_assign_op`
(`OpDatabase(vdb) ; v = OpGetField(vdb,…) ; OpAppendVector(v, src.f)`) and, when
`proves_borrow(v)` holds, **replace it with the borrow form** (`v = OpGetField(src,…)`,
dep on `src`, `skip_free`) — the exact shape `enemies = s.enemies` already emits.

This keeps the decision in ONE place (the analysis phase), makes the conservative copy
the default (parse), and the elision a proven subtraction — the cleanest possible
cut-in.

---

## 7. Consumer map — the "multiple locations" this unlocks

| Consumer | Today | After the pre-work |
|---|---|---|
| `#415`/p379 field-read copy (`parse_assign_op`) | shape rule (base `Reference` + empty deps) | `proves_borrow(v)` query (Tier 0/1) |
| `#465` return delivery (`classify_vector_delivery`) | deps + tail-shape, re-derived | `is_owner`/`borrow_source` query (Tier 3) |
| "& never modified" diagnostic (`parser/mod.rs` `written`) | its OWN mutation walk | consume `mutated` (de-dup the second walk) |
| copy-bottleneck lint (elision §5) | — | `is_owner` + `source` to flag elidable copies |
| `lastuse_reclaim` liveness (`scopes.rs`) | own liveness | shares the same walk/live-range substrate |

The `written`-set de-dup is itself a worthwhile early win: it removes a whole separate
mutation walk and makes the diagnostic and the optimizer agree **by construction** (they
read the same fact) — no risk of the lint saying "elidable" while the diagnostic
disagrees.

---

## 8. Coverage & soundness — the load-bearing proof (probe this hardest)

The invariant (§2) rests on **"unrecognized use ⇒ mutated = escapes = true."** Two ways
that can be wrong, both must be closed:

1. **A use-shape that mutates/escapes but the walk reads as benign** → false
   `!mutated`/`!escapes` → wrong borrow → UAF. This is the only way the phase causes a
   bug. Defense: the use-classifier is an **exhaustive match over `Value` variants**
   (the H7 IR-codec round-trip pattern) — a new variant that can carry a write/escape
   fails to compile until classified, and the *default arm is conservative*
   (`mutated = escapes = true`). So a forgotten shape is safe (copies), and a *silently
   benign-looking* shape is caught by the exhaustive arm.
2. **Aliasing the analysis can't follow** (a write reaching `v` through an alias the walk
   didn't connect — the probe9 `cells = sc.field; cells[i]=…` shape). Defense: any var
   that is itself borrowed-from / aliased is conservatively `mutated` unless the alias
   chain is fully tracked; Tier 0 sidesteps this entirely by only proving borrow for a
   **param** source with a **directly read-only** local (no alias chain).

**The coverage sentinel:** a debug assertion + a `tests/scripts` corpus where every
`Value` variant that can write or escape a var appears at least once, each asserting the
var is classified `mutated`/`escapes`. This is the "prove coverage" half of
reify → prove → cut over — it keeps the analysis honest as the IR grows.

---

## 9. Falsification probes (design-protocol — before code)

| Claim | Cheapest falsifying probe | Required |
|---|---|---|
| conservative default is safe | feed an unclassified synthetic use; assert it reads `mutated` | profile = mutated (copies) |
| `mutated` catches element/append/`&`-arg | `v[i]=…`, `v += …`, `f(&v)` | each ⇒ `is_owner` true |
| `escapes` catches return/store/capture | `return v`, `x.f = v`, closure capturing `v` | each ⇒ `is_owner` true |
| read-only param-field ⇒ borrow proven | `tile_at` shape | `proves_borrow` true, IR is borrow form, no leak, both backends |
| **the cleanest claim (attack):** "one walk computes all facts soundly" | a use reachable only through an alias chain (`v = a; a[i]=…`) | analysis must classify `v` mutated OR refuse to prove borrow — never a wrong borrow |

The last row is the over-unification guard: the seductive claim is that a single linear
walk suffices. It does **not** for alias chains — so the honest scope is "prove borrow
only where the alias graph is trivial (Tier 0), widen with explicit alias tracking
later," never "the walk handles everything."

---

## 10. Re-assertion count & non-goals

- **Producer N = 1** (the one walk in `scopes::check`). Consumers are pure queries, so
  they add no re-assertion sites — the opposite of today's spray. The risk to watch
  (step 2): a consumer that *keeps* re-deriving instead of querying — the consumer map
  (§7) is the migration checklist; "still re-derives" is the failure to flag.
- **Non-goal:** a complete ownership/borrow checker. This is a *sound, incomplete*
  use-analysis whose incompleteness is always safe (copies). It is a stepping-stone
  toward the OWNERSHIP_MODEL `deps` system, not its finished form.
- **Non-goal:** changing value semantics or parse-time emission. The phase only
  subtracts proven-safe copies.

---

## 11. Build order (the pre-work, then the tiers consume it)

1. `UseProfile` struct + the full-IR use-classifier in `scopes::check` (exhaustive
   conservative match) + the query API (§3–5). Land with the coverage sentinel (§8) and
   the §9 probes — **no behaviour change yet** (nothing consumes it; profiles computed,
   asserted, unused).
2. De-dup the "& never modified" diagnostic onto `mutated` (§7) — first consumer, pure
   refactor, byte-identical diagnostics, proves the API.
3. Tier 0 elision rewrite (§6) — first *optimizing* consumer; the elision matrix +
   crawler `surfacetest` speed gate (copy-elision-design §9).
4. Tiers 1–3 widen `UseProfile` (source-dominance, ¬D2, return-delivery unification),
   each gated by its own matrix.
