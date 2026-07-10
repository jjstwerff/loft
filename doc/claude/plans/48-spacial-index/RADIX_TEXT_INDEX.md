# @PLN48 — the text-index use case for `radix<T[text]>`

**Status:** design / justification note. The consumer that earns a loft-visible
`radix<T[text]>`, and the reference workload for building + testing it. No code yet.

This complements [RADIX_TREE.md](RADIX_TREE.md) (the tree) and its §8 (whether raw
`radix` should be loft-visible). RADIX_TREE.md §7.1 flags the text-key path as
*"designed but untested — no test yet builds a text oracle."* This note names the
real program that exercises it.

---

## The workload — a symbol / tag index that grows with the project

`tools/indexer/src/scan.loft` builds `index/tags.json`: it scans the tree for
`@`-tags (`@P259`, `@PLN48`) and doc links, and per tag emits the list of places it
occurs. Today it keys a distinct, alphabetically-ordered set with
`sorted<TagSlot[name]>` (`DistinctSets`), at ~5 200 refs over ~620 distinct keys —
sub-second.

The goal is to scale this to a **full game engine**: many libraries, growing over
time, so the same shape at 10–100× — hundreds of thousands of refs over tens of
thousands of distinct tags/symbols. The *same* structure a language server's symbol
index needs (§ [LSP](../../COMPILER.md)); the indexer is its first, already-shipping
instance.

## Two quadratic walls at scale

Both confirmed in the current code:

1. **Distinct-set build is O(M²).** `sorted<T>` is a sorted *vector*
   (`database/mod.rs` — "Sorted vector"), so each insert shifts the array — O(M).
   M inserts → O(M²).
2. **Bucket emit is O(N×M).** `emit_tag_bucket` re-scans all N rows once per distinct
   tag: `for r in rows { if r.tag == tag }`.

At N≈5 200 / M≈620 both are trivial. At N≈500 000 / M≈50 000: build ≈ 2.5×10⁹,
emit ≈ 2.5×10¹⁰ — minutes, on something meant to run on every `make index`.

## The scaled design — group with a hash, order with a radix

```
hash<TagBucket[tag]>    group refs by tag in ONE O(N) pass  → kills the O(N×M) emit
radix<TagSlot[name]>    the distinct, alphabetical key set  → O(N·k) build, ordered + prefix
```

Both are needed **because they do different jobs**:

- The hash kills the *dominant* term (the O(N×M) emit): one O(N) grouping pass, then
  emit each bucket directly — O(N + M). This is the bigger win and it is a hash job,
  not a radix job.
- The radix supplies the *ordered* half a hash cannot: alphabetical emit, and the
  `prefix:` query (`idx prefix:@PLAN22`) moved out of the bash/jq wrapper into loft as
  a `seek` + walk-while-prefix.

**Do not mistake #1 for a data-structure problem.** Swapping only the distinct set to
radix fixes the *smaller* quadratic and leaves the O(N×M) emit dominating. The
grouping pass is the first-order fix.

---

## Do we need two *types*? — hash **and** radix

Yes, at the language level, and the reason is not "radix can't do it."

Radix **functionally subsumes** hash: a radix keyed on an exact key answers
membership in O(key length) — it *can* do everything a hash does. Hash does **not**
subsume radix: it is unordered, so it cannot do prefix or ordered iteration.

So the split is not about capability, it is about the **constant factor**, and that
factor is fundamental and workload-driven:

| | exact lookup | ordered / prefix | the analogue |
|---|---|---|---|
| `hash<T[k]>` | **O(1)** | — | Rust `HashMap`, C++ `unordered_map` |
| `radix<T[k]>` | O(k) | **yes** | a trie / ordered map |

Every serious language keeps both (`HashMap` **and** `BTreeMap`) for exactly this
reason: collapsing to one taxes every O(1) membership check with an O(k) descent.
The hot grouping pass over N rows wants O(1); only the M distinct keys need order. So
**keep both types.**

### Is radix then redundant with `sorted` / `index`?

No — it fills a slot the existing ordered kinds only half-cover:

| ordered kind | insert | prefix | cheap local move | best for |
|---|---|---|---|---|
| `sorted<T>` | O(M) (vector shift) | via lower-bound | no | small, static, read-heavy, cache-dense |
| `index<T>` | O(log M) (RB-tree) | via lower-bound | no | general ordered map |
| `radix<T>` | O(k) | **native** | **O(log Δ) finger** | text prefix, spatial, high-churn ordered |

`radix` earns its place through **native prefix**, the **finger move** (re-index one
changed file in O(log Δ), not rebuild M), and **Morton/spatial** — none of which
`sorted`/`index` give. For a generic ordered map with no prefix/finger need, `index`
stays the default; the language should *guide*, not force. `radix` is not redundant,
but it does overlap, so its docs must say when to reach for it.

## Do we need two *instances* here? — a measurable choice

Within the indexer, there is a genuine fork, and this is what "build and test both"
resolves:

- **(A) hash + radix** — `hash<TagBucket[tag]>` for grouping, `radix<TagSlot[name]>`
  for the ordered set. Build O(N + M·k): N *cheap* O(1) hash hits plus M radix
  inserts. Two copies of the key, more code.
- **(B) one radix multimap** — `radix<TagBucket[tag]>` whose leaf *holds* the rows.
  The leaf is the bucket, so one structure groups **and** orders **and** prefixes.
  Simpler, one copy of the key. Build O(N·k): every one of the N rows pays an O(k)
  descent, most of them only to hit an existing tag.

Prediction to test: **(B) is simpler and fine up to moderate scale; (A) pulls ahead
once N ≫ M**, because (B) pays O(k) per *row* where (A) pays O(1) per row and O(k)
only per *distinct* tag. At N/M ≈ 10 (today) the gap is ~10× on the grouping pass;
whether that matters depends on absolute size. Measure both, keep the simpler one
until the numbers say otherwise.

## Build-and-test plan

Depends on S2 exposing `radix<T[text]>` (rides the shared `Spacial` runtime with a
text oracle — no second runtime variant; RADIX_TREE.md §8).

1. Land the text oracle (the currently-untested path, RADIX_TREE.md §7.1) and a
   `radix<T[text]>` surface keyword aliasing the shared kind.
2. Convert `scan.loft`'s `DistinctSets` from `sorted<TagSlot[name]>` to
   `radix<TagSlot[name]>` — the first real dogfood of the text radix.
3. Add the `hash<TagBucket[tag]>` grouping pass; emit in O(N + M).
4. Move `idx prefix:` from the bash wrapper into loft (`seek` + prefix walk).
5. Benchmark A vs B at 1×, 10×, 100× the current corpus; keep the winner.

Acceptance: `make index` stays well under a second at the current corpus and scales
sub-quadratically as libraries grow; `prefix:` answered in loft; the text-radix path
exercised by a shipping tool.
