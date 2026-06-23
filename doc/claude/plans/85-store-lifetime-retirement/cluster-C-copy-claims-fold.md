<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster C / H10 — fold `copy_claims` source enumeration onto the keystone

Plan id: [@PLN85](https://github.com/loft-lang/plans/issues/85) · companion to
[STABILITY_HOTSPOTS.md § H10](../../STABILITY_HOTSPOTS.md) and
[STABILITY_REDFLAGS.md § cluster C](../../STABILITY_REDFLAGS.md).

## What this is

The heap-cascade walk — "for each owned child of a record, at which stride, of which
type" — is encoded once in the keystone `Stores::for_each_owned_child` (in
`src/database/allocation.rs`, around line 94). `remove_claims` (around 1926) already
reads it. The `copy_claims` family does not yet: its four per-kind helpers each
re-roll the same source walk by hand. That divergence is the densest historical bug
cluster in the tree (@P290 SIGSEGV, @P306/@P318 hash slot-drift, @P309 missing length
header). This plan carries the source walk **once** for copy too.

## What folds and what does not

The walk has two halves. **Source enumeration** — "list this record's child slots" —
is the shared fact; it folds onto the keystone. **Destination build** — "allocate the
copy into the `to` store" — is genuinely per-kind and stays in each helper. Do not
touch it: unifying the destination is how @P318/@P309 come back.

`copy_claims_hash_body` (around 1474) is the worked template: it already reads
`for child in self.for_each_owned_child(rec, tp).children`, takes `child.owning_elem`
as the source element record, and pairs each with a freshly-claimed destination slot
(`claim` → back-pointer → `copy_block` → recurse → `hash::add`). The other three copy
that shape, with their own source loop instead of the keystone.

| helper (≈line) | source walk to replace | destination build that STAYS |
|---|---|---|
| `copy_claims_index_body` (1542) | `collect_index_nodes(rec, left)` — the **same call** the keystone Index arm makes | `tree::add` re-insert; ≈40 lines, already mirrors `hash_body` |
| `copy_claims_array_body` (1404) | `for i in 0..length { elm = get_u32_raw(cur, 8+4*i) }` | @P309 length-header `set_u32_raw(into, 4, length)`; per-element slot-copy |
| `copy_claims_seq_vector` (1360) | `for i in 0..length { pos: 8 + size*i }` | one bulk `copy_block(length*size+4)`; positional slot-copy |

Each source walk is ~3–6 lines and matches the keystone position-by-position (verified:
Vector `8+size*i`, Array `8+4*i`, Index the identical `collect_index_nodes`). So the
fold is mechanical, with **one wrinkle**: `array_body` and `seq_vector` are called with
the *content* type (`*v`), but the keystone wants the *container* type. Folding them
needs a small call-site/signature change (the calls are around lines 1655 and 1703),
not a pure body edit. `index_body` already takes the container type, so it is a near
drop-in.

`record_new` / `record_finish` (construction) are a separate WRITE/build path and do
**not** fold here — see [STABILITY_HOTSPOTS.md § H10](../../STABILITY_HOTSPOTS.md).
`validate_claims` also does **not** fold (it is a defensive walk that bounds-checks
each pointer before dereferencing; the keystone trusts pointers). That boundary is
pinned in the keystone's own doc comment (`OwnedChild` in `allocation.rs`).

## The verifiable phased plan

One helper per phase, each independently shippable. Prove green on **both backends**
before editing and after each phase; on any red, revert that one site and diagnose
before continuing (bisect-by-site). `B=./target/release/loft`,
`T=tests/scripts/85-store-lifetime-claims-keystone.loft`.

### Phase 0 — baseline (no edits)
Confirm the current tree is green so any later red is unambiguously the fold.
- `$B --interpret --tests $T` → ok
- `$B --native --tests $T` → ok
- `LOFT_COPY_CHECK=1 $B --interpret $T` → no `copy_check` mismatch warning
- `cargo test --release --test leak` → pass

### Phase 1 — fold `index_body` (lowest risk)
Replace the `collect_index_nodes` source walk with the keystone children, reading
`child.owning_elem` as the source node; keep the `tree::add` destination body unchanged.
- Verify: rebuild; `$T` on interp **and** `--native` → ok (same values, no panic);
  `cargo test --release --test leak` → pass; `LOFT_COPY_CHECK=1` on `$T` → clean;
  `tests/scripts/62-index-range-queries.loft` + `129-sorted-index-field-deepcopy.loft`
  on both backends → ok.

### Phase 2 — fold `array_body`
Change the call site / signature to pass the container type; replace the `8+4*i` source
loop with keystone children. **Keep** the @P309 length-header write and the rest of the
build.
- Verify: rebuild; `$T` both backends → ok; leak gate → pass; `LOFT_COPY_CHECK=1` →
  clean; `374-vector-hash-sibling-dup-key.loft` → ok.

### Phase 3 — fold `seq_vector`
Same container-type adjustment; replace the `8+size*i` source positions with keystone
children. **Keep** the single bulk `copy_block` (iterate the keystone alongside it —
accept the double pass; do not merge them).
- Verify: rebuild; `$T` both backends → ok; leak gate → pass; `LOFT_COPY_CHECK=1` →
  clean; `182-deep-nested-vector-copy.loft`, `183-nested-single-vector.loft`,
  `152-i319-i320-field-vectors.loft`, `163-plan53-cross-store-vector-add.loft` → ok on
  both backends.

### Phase 4 — full verification + docs
- `./scripts/find_problems.sh --bg` then `--wait` → only the known `StableCrateId`
  native-cdylib env failures, nothing new.
- `cargo fmt --check` clean; `cargo clippy --release --lib` clean.
- Update the keystone `OwnedChild` comment: the three helpers "are a mechanical
  source-fold away" → "now fold". Bump the H10 register: copy-source fold landed;
  construction stays separate.

## Done when

`for_each_owned_child` is the single source enumeration for `remove_claims` and all
four `copy_claims` kinds; the keystone guard, the leak gate, and `LOFT_COPY_CHECK` are
green on interp **and** `--native`; the suite shows no new failures. The three
divergent re-encodings of the cascade walk are gone, with destination build correctly
left per-kind.

## Guards

`tests/scripts/85-store-lifetime-claims-keystone.loft` covers every axis (vector, hash,
sorted/ordered, index + sibling-sorted = the @P309 axis, multi-heap struct, inline
enum) under the store-leak gate. `cargo test --release --test leak` catches a dropped
or double-freed element record. `LOFT_COPY_CHECK=1` (or `LOFT_LOG=copy_check`) is the
in-process tripwire for an off-by-one source fold: it walks source and destination
lengths in parallel and warns on any nested-collection mismatch.
