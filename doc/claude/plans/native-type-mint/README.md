<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Native duplicate-type-mint — a warm-cache `--native` panic

**Status — FIXED.** `install_schema` now restores the constructor lookup key of a
promoted collection ([`Stores::promoted_lookup_key`], `src/database/types.rs`).
Guarded by two unit tests + the probe set here.

```
thread 'main' panicked at src/database/types.rs:
index out of bounds: the len is 75 but the index is 75
```

`--native` only; the interpreter is correct in every case seen.

---

## 0. The correction that unlocked this — it was never random

The first investigation recorded the fault as **per-process random** and built the
repeat harness around that belief. It is not random. It is **deterministic on the
whole-program cache**:

| run | verdict |
|---|---|
| 1st (cold — the cache is being written) | ok |
| every later run (warm — the cache is read back) | bad |

`repeat.sh` cleared the per-directory `.loft` cache before every run, but the
whole-program bundle lives in `$XDG_CACHE_HOME/loft` (`cache::program_cache_paths`),
which it never touched. So the harness measured cold-once-then-warm-forever and
reported it as a 1-in-12 random ratio.

Two one-line probes settle it, and either would have been worth more than the
mechanism trace that came first:

```bash
rm -rf ~/.cache/loft/program-*   # then 4 runs -> ok, BAD, BAD, BAD   (not random)
LOFT_NO_CACHE=1                  # 6 runs      -> ok x6               (the cache IS the fault)
```

**The lesson to carry:** "random" was a conclusion drawn from a ratio, and the ratio
came from an instrument that was not clearing the state it thought it was. Before
theorising about a random fault, check that each run really starts from the state
you believe — `1/N` is the signature of *first run differs*, not of randomness.

## 1. Mechanism

`names` maps a type's **constructor spelling** (what `Stores::vector` / `sorted`
render) to its id — that is how those calls dedupe. `Stores::finish` then PROMOTES a
collection whose content type is shared by more than one container and renames it in
place: `vector<X>` → `array<X>`, `sorted<X[k]>` → `ordered<X[k]>` (`types.rs`,
`finish_type`). The rename does not touch `names`, so on a **cold** run the
pre-promotion spelling stays the live lookup key and everything resolves.

`install_schema` (the warm-load path) rebuilt `names` purely from the stored type
names — which for a promoted collection is the POST-promotion spelling. The
constructor key was therefore gone, so the next `vector`/`sorted` call missed and
minted a **second** type for a collection that already existed. That id was baked
into the emitted native code while `init()` never registered it → the runtime type
table is shorter than the id it is indexed with → the panic.

`LOFT_TRACE_MINT=1` shows it directly (good run left, bad run right):

```
vector "vector<Node>"     hit=67  len=74     vector "vector<Node>"     MINT=74 len=74
sorted "sorted<Node[id]>" hit=71  len=74     sorted "sorted<Node[id]>" MINT=75 len=75
hash   "hash<Node[id]>"   hit=72  len=74     hash   "hash<Node[id]>"   hit=72  len=77
```

`hash` and `index` hit in both — they are never renamed. Only the two promoted
spellings miss, which is the whole fault in one line.

## 2. The fix

`install_schema` registers the constructor spelling as an alias alongside the stored
name. Promotion rewrites only the leading word and leaves the `<content[key]>` tail
byte-identical (`key_name` and `create_key` render that tail the same way), so
recovering the original key is an exact prefix swap, not a re-render. Aliases are
applied **after** the primary map and never overwrite, so a schema that also holds a
genuinely unpromoted `vector<X>` keeps its own id.

## 3. Why attempt 1 failed, and why this differs

The earlier attempt put the same string surgery at the **constructors** — if
`sorted<X[k]>` misses, try `ordered<X[k]>`. Measured: `graph_plus_wide` unchanged at
1/20 and `keyed_local_promoted` REGRESSED from 12/12 to 1/20. It changed the COLD
path too, where a genuinely-new unpromoted collection must mint its own type rather
than resolve onto a promoted one.

This fix is at `install_schema`, which runs only on the warm path, and it restores
exactly the mapping the cold run had at that point. Lookup and reconstruction both
read the CURRENT content-type name, so they agree by construction.

## 4. Measurement

`repeat.sh` clears `.loft`, counts VACUOUS runs separately, and carries a control
probe that must never pass. It does **not** clear `~/.cache/loft` — clear that by
hand when you want a cold reading.

```bash
./repeat.sh 20                      # working-tree binary
./repeat.sh 20 /usr/local/bin/loft  # a pre-change binary, for before/after
```

| probe | shape | before | after |
|---|---|---|---|
| `graph_plus_wide` | `Graph{vector,index}` **+** `Wide{sorted,hash}` over one element | ok=1/12 | **ok=20/20** |
| `keyed_local_promoted` | `Wide{sorted,hash}` + keyed locals | ok=12/12 | ok=20/20 |
| `keyed_local_only` | one `sorted` local | ok=12/12 | ok=20/20 |
| `zz_control` | must never pass | ok=0 ✓ | ok=0 ✓ |

The Rust guard is `install_schema_keeps_the_lookup_key_of_a_promoted_collection`
(`src/database/types.rs`), verified non-vacuous: with the alias removed it fails with
the same duplicate id the panic came from.

## 5. Related, already fixed (do not confuse with this)

- `498a7027` — a struct literal emptied every vector field but the first (#437).
- `6b629e07` — an ordered-field element copy dropped the last field of every record
  (`sorted` beside `hash`). That one was this shape's sibling and is why
  `keyed_local_promoted` was already green.

## 6. Scope

Any program whose schema promotes a collection, run twice with the program cache on
(the default). Reachability looked narrow because the first run always works — the
same "baked id vs runtime table" family as #483.
