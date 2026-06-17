<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `lib/cache/` — mtime-invalidated read-through cache

**Status:** Future — opened 2026-05-15 from @PLAN35 viewer
hot-path observation.

## Why

The viewer (`tools/viewer/src/main.loft`) reads
`index/tags.json` (~150 KB) from disk on EVERY request and
re-parses it via `json_parse`.  The dashboard hits 5+ JSON
reads per page render.  With page latency dominated by
JSON parsing, a per-file mtime-keyed cache cuts request
time roughly in half.

Same shape would help any future loft tool that re-reads
configuration, data files, or computed indexes on every
operation.

## Surface

```loft
struct Cache<V> {
  // Opaque — implementation detail
}

pub fn cached_text(c: Cache<text>, key: text, loader: fn(text) -> text) -> text;
pub fn cached_json(c: Cache<JsonValue>, key: text, loader: fn(text) -> JsonValue) -> JsonValue;
```

The cache key is a file path; the cache holds the loader's
result, invalidating when the file's mtime advances past the
cached entry's stored timestamp.

Usage in the viewer:

```loft
fn idx_cache() -> Cache<JsonValue> { /* singleton via closure */ }

fn page_welcome() -> text {
  idx = cached_json(idx_cache(), "index/tags.json",
                    fn(p) { json_parse(file(p).content()) });
  // ... use idx.field("problems_open") etc. as today ...
}
```

## What ships

- `lib/cache/src/cache.loft` — the API above.
- Implementation: per-key `(stored_mtime, value)` pair stored
  in a hash<>; loader fires only when the file's current
  mtime > stored.
- File-stat primitive: `file(path).mtime() -> integer` (a
  small stdlib gap — currently file() exposes format / size
  but not mtime).

## Consumer changes once shipped

- viewer's `page_landing` / `page_welcome` / `page_tag` move
  their `file("index/tags.json").content()` reads to
  `cached_json(...)`.  Per-request latency drops from O(parse
  150KB) to O(stat).
- viewer's git-state JSON reads (`branch.json`, `commits.json`,
  `changed.json`) get the same treatment.

## Effort

S.  ~50 lines of loft.  The mtime primitive is a small
stdlib add (XS).

## Cross-references

- [@PLAN35](../../plans/finished/35-branch-review-viewer/README.md)
  — driver use case (hot-path JSON re-read).
- [STDLIB.md § Open work](../../STDLIB.md#open-work)
  — sibling stdlib gaps surfaced by the same dogfood pass.
