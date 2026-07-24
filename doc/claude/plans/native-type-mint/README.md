<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Native duplicate-type-mint — a per-process random `--native` panic

**Status — NOT FIXED. Diagnosed to the mechanism, one fix attempted and REVERTED
(it made things worse, measured).** The tree is clean; only this directory is new.
The harness in here is the durable part — build on it, do not re-derive it.

```
thread 'main' panicked at src/database/types.rs:
index out of bounds: the len is 75 but the index is 75
```

`--native` only; the interpreter is correct in every case seen.

---

## 0. READ THIS FIRST — the trap that cost most of the last session

**The fault is per-process random. A single run is worthless as evidence.**

I called this "fixed" twice on single-run readings and was wrong both times; I
also once counted a run that produced *no output at all* as a pass. Both are
recorded here so the next person does not repeat them:

- a lucky variant reads as a pass → always run **N≥12** and report a ratio;
- a stale `.loft/cache` binary re-runs an OLD compile → clear it **before every
  run**;
- a compile failure prints nothing, and `grep -c` then returns 0 → count
  **VACUOUS** separately, never as a pass.

`repeat.sh` does all three, and carries a control probe that must NEVER pass
(`zz_control`) so a blind harness announces itself.

```bash
./repeat.sh 20                      # working-tree binary
./repeat.sh 20 /usr/local/bin/loft  # a pre-change binary, for before/after
```

## 1. Baseline (HEAD = 7c08cf0d, N=12–20)

| probe | shape | verdict |
|---|---|---|
| `graph_plus_wide` | `Graph{vector<Node>, index<Node[id]>}` **+** `Wide{sorted<Node[id]>, hash<Node[id]>}` + keyed locals | **ok=1 bad=11** |
| `keyed_local_promoted` | `Wide{sorted,hash}` + keyed locals, no `Graph` | ok=12 bad=0 |
| `keyed_local_only` | one `sorted` local | ok=12 bad=0 |
| `zz_control` | must never pass | ok=0 ✓ |

So the fault needs **both** a struct minting `array`/`index` types **and** a
struct minting `sorted`/`hash` types over the same element.

## 2. The mechanism, as far as it is established

Traced by printing every `Stores::sorted()` call (name, whether found, table
length) across good and bad runs — the diff between them is unambiguous:

```
GOOD:  [sorted] name="sorted<Node[id]>" found=None      types.len=71   -> mints at 71 (correct)
BAD:   [sorted] name="sorted<Node[id]>" found=None      types.len=75   -> mints a DUPLICATE at 75
       [sorted] name="sorted<Node[id]>" found=None      types.len=71
       [sorted] name="sorted<Node[id]>" found=Some(71)  types.len=74
```

A bad run makes an **extra** `sorted()` call against a 75-entry table in which
the name is not registered, so it mints a second type for a collection that
already exists at 71. That id is then baked into the emitted op while `init()`
never registers it → the runtime table is shorter than the id → panic.

**Why the name is missing.** `Stores::finish()` PROMOTES a collection whose
content type is LINKED (shared by more than one container) and renames it in
place — `sorted<X[k]>` → `ordered<X[k]>`, `vector<X>` → `array<X>`
(`types.rs` ~396–410). The rename does not touch `self.names`. Separately,
`install_schema` (`types.rs:65`, the program-cache / schema-load path) rebuilds
`names` **purely from the current type names**, so after a schema load the
pre-promotion spelling is gone entirely.

That is a real inconsistency and almost certainly part of the story — but see §3,
because acting on it directly made things worse.

## 3. Attempt 1 — REVERTED, and why it failed (do not repeat verbatim)

Added an inverse lookup at the two constructors: if `sorted<X[k]>` misses, try
`ordered<X[k]>`; if `vector<X>` misses, try `array<X>`. The spellings differ only
in the leading word, so the string surgery was exact.

Measured with the harness:

| probe | before | after |
|---|---|---|
| `graph_plus_wide` | ok=1/12 | ok=1/20 (unchanged) |
| `keyed_local_promoted` | **ok=12/12** | **ok=1/20 — REGRESSED** |

So the two names are **not** interchangeable. Promotion is a property of one
schema instance, and mapping the old spelling onto the promoted type changes
which id a context resolves to. Reverted (`git checkout -- src/database/types.rs`).

**The lesson to carry:** the duplicate mint is a SYMPTOM. Two schema instances
disagree about whether the type is promoted; forcing the name lookup to agree
just moves the disagreement. Find why one instance is promoted and the other is
not.

## 4. Where to go next — highest value first

1. **Identify the two callers.** The trace shows `sorted()` hit against tables of
   length 75, 71 and 74 in one process — those are different `Stores` instances
   (clones), not one growing table. Put a backtrace (or a caller tag) on each
   `sorted()` call and find out which pipeline each belongs to: the parse
   database, the `byte_code` clone, the native emitter's `self.stores`, and the
   schema-load path are the candidates. **Until you know which instance mints the
   duplicate, any fix is a guess** — that is exactly how attempt 1 went wrong.
2. **Find what makes it random.** The variance correlates with a schema load
   having happened (`install_schema` rebuilding `names`). Confirm that: force the
   program-cache path on and off and re-measure with `repeat.sh`. If it is the
   cache, the fault is "a cached schema loses the pre-promotion name", which is a
   much narrower and more honest target than the whole type table.
3. **Then fix the disagreement at its source**, not at the lookup — most likely
   either by making the promotion preserve the lookup key (rename only a display
   name), or by making the schema round-trip carry `names` rather than
   reconstructing it (`install_schema`'s own doc comment already flags that
   reconstruction as approximate: *"P379 library-qualified names would need the
   names map stored separately"* — the same weakness, already known).
4. **Re-measure with `repeat.sh` at N≥20 on every probe, both before and after**,
   and add a probe for any new shape you touch. A fix is only real when
   `graph_plus_wide` is ok=N and the other two have not moved.

## 5. Related, already fixed (do not confuse with this)

Two defects found in the same area and shipped separately — this one is what is
left after them:

- `498a7027` — a struct literal emptied every vector field but the first (#437).
- `6b629e07` — an ordered-field element copy dropped the last field of every
  record (`sorted` beside `hash`). That one WAS this shape's sibling and is
  fixed; it is why `keyed_local_promoted` is green at HEAD.

## 6. Scope note

Reachability is narrow: it needs one struct with `vector` + `index` fields and
another with `sorted` + `hash` fields over the same element type, compiled with
`--native`. No consumer has reported it; it was found by widening the Commonstore
handoff verification. It is a genuine silent-corruption-class hazard though — the
same "baked id vs runtime table" family as #483 — so it should not be left open
indefinitely.
