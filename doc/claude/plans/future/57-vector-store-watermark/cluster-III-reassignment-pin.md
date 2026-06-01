# Cluster III — reassignment does not free the overwritten store

## Shape

Reassigning a vector local — `v = [new]` when `v` already holds a vector — does **not**
free `v`'s previous store. Every overwrite pins the now-unreachable old value until the
function scope exits, so a straight-line sequence of reassignments accumulates one pinned
store per overwrite.

## Severity (two fields)

- **Corruption / panic / hang:** none.
- **Leak (escapes scope):** **none** — the overwritten stores free at scope exit; the
  exit gate passes. Watermark concern only — O(reassignments) extra live stores.

## Verified

- ✅ **Overwrite pins the old store.** Probe 14: one local reassigned 10× → 14 allocs,
  all freed at scope-end (`aaa…fff`, zero interleaving). Each `v = [new]` allocates a new
  store and leaves the prior one alive. *(trace: RESULTS.md probe matrix row 14.)*
- ✅ **Loops are exempt.** The same per-iteration `v = [..]` inside a loop reuses a single
  store in place (probe 02) — the overwrite-pin is a *straight-line* phenomenon.

## Hypothesized (Stage B — needs source reading)

- 🤔 **Assignment to an existing store-backed local does not free the prior DbRef before
  rebinding.** The store-free is deferred to the scope sweep instead of firing at the
  overwrite. **Action:** read the assignment codegen for `existing_local = <new vector>`;
  confirm no `OpFreeRef` of the old DbRef is emitted at the overwrite point.
- 🤔 **Shares cluster I's root + fix surface.** Both are "store-free anchored only at scope
  exit, never when the store becomes dead." Cluster III's dead value is *unreachable*
  (stronger than cluster I's dead-but-named), but freeing it early needs the **same
  aliasing guard**: an overwritten store aliased by a still-live binding must not be freed.

## Fix-safety — resolved

The aliasing question for clusters I/III was answered during the @P394/@P395 edge probing:
loft vectors are **copy-semantics** on expression-assignment (`b = a[..]`; then `b[0] = 99`
leaves `a[0] == 1`). Plain locals are therefore **not** aliased to each other — only
explicit `&vector` params alias. So freeing a dead/overwritten local's store is safe except
across an explicit `&` borrow, which is already tracked. This removes the main risk from the
cluster I/III fix.

## Fix options (Stage C)

1. **Free the old store at the overwrite** (emit `OpFreeRef` of the prior DbRef before the
   new store is bound), guarded by the `&`-borrow check. Folds into cluster I's last-use
   freeing — do them together.
2. **Do nothing** (benign) and rely on the heuristic-floor fix to silence the warning.

## Probes

`14` (overwrite pin); contrast `02` (loop reuse).
