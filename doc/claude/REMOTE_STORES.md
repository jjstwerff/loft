<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Serving static data over HTTP range

How a loft program reads **part** of a large file that lives on a web server,
fetching only the bytes a lookup touches instead of downloading the whole thing.

The shape this is for is ordinary and not map-specific: **one big immutable
dataset, many small reads, no server-side code.** A game's world chunks, meshes
and sounds; a product catalogue; a gazetteer; a dictionary; a parts list; a
sensor archive; a map block — anything you can build once, upload as a static
file, and then query. The dataset sits on whatever serves static files (a CDN,
S3, GitHub Pages, nginx), and the client pulls pages of it on demand.

**No database server and no API.** The file is the API; HTTP range is the
protocol; the server is a dumb file host. That is the whole trade this buys.

## Games: asset streaming without an asset server

This is the shape a game already has, and loft is games-first, so read the rest
of this document with that case in mind. **World chunks, meshes, textures,
sounds, animations, dialogue, level data** — built once by a tool, packed into a
store, uploaded as a static file, and pulled in as the player moves.

```loft
// The world is one 2 GB pack on a CDN. Load the chunks around the player.
chunks: hash<Chunk[cell]> = [];
_ = store_load_keys(chunks, "https://cdn.example/world.store", ring_around(here));
```

What it replaces is an asset server, a streaming service, and the deployment that
goes with them. A browser game built with `--html` gets the same call: the world
streams from static hosting, and the program is written as though the pack were
on local disk.

Three things follow from the cost model below, and they are the ones that decide
whether a game feels good:

- **Pack by locality, because a page is shared.** Neighbouring chunks in the same
  64 KiB page arrive for the price of one; an animation's frames, a mesh and its
  LODs, a sound and its variants all want to be adjacent in the file. This is a
  property of how the pack is BUILT, and it is worth more than any runtime knob.
- **Prefetch ahead of the player, never on demand at the point of use.** A round
  trip is ~40 ms and you cannot take one inside a frame. Load the ring around the
  player, not the cell under them.
- **Batch the ring.** `store_load_keys` opens the reader once and reuses its page
  cache, so a ring is one traversal rather than N; `store_load_range` does the
  same for a contiguous span of spatial keys.

Split the pack the way the reads split: assets needed on **every** frame from the
first (the UI atlas, the player mesh, core sounds) are small and universal, so
load them **whole** at boot with `store_load_url_trusted`. The world is large and
read sparsely, so page it. Mixing those two in one file makes the boot load pay
for the world.

## The two ways to read a remote store

| | fetches | use when |
|---|---|---|
| `store_load_url_trusted(r, url)` | the **whole** image, once | it is small, or you need all of it anyway |
| `store_load_url(r, url, sha256)` | the whole image, **pinned** to a hash | the origin is not trusted |
| `store_load_key(local, url, key)` | only the pages that ONE lookup touches | the dataset is large and reads are sparse |
| `store_load_key_text(local, url, key)` | same, for a text-keyed hash | |
| `store_load_keys(local, url, keys)` | several keys, reader opened once | a batch — cheaper than N calls |
| `store_load_range(local, url, lo, hi)` | a contiguous key range | ordered collections |

The four paged forms take **a local path or an `http(s)://` URL, interchangeably**,
on every target including the browser (`--html`). Nothing in the program changes
when the data moves from disk to a CDN — which is what makes "develop against a
local file, ship against a URL" work.

There is a third way, for when the program should not have to say which entries it
wants: `store_bind_lazy` binds a collection to one of these sources once, and every
lookup that MISSES fetches its own entry ([STDLIB.md § Lazy store
binding](STDLIB.md), `@F108`). Same reader, same pages — driven by the lookups
instead of by a call.

```loft
// The whole dataset is 800 MB on a CDN. This reads a few pages of it.
parts: hash<Part[id]> = [];
if !store_load_key(parts, "https://cdn.example/catalogue.store", 91_337) {
  println("no such part");
  return
}
```

`local` starts EMPTY and ends holding only what was asked for. The entry's own
fields are **relocated** into the local store, so `text`, nested structs and flat
vectors all come across. A `vector<text>` or a `vector<vector>` is refused — its
element pointers would dangle — and the refusal says so on stderr rather than
looking like an absent key.

## What the server has to do

Almost nothing, which is the point. Two requirements:

1. **Answer `Range: bytes=<off>-<last>` with `206 Partial Content`.** Every
   static file server does this by default; it is the same mechanism video
   seeking uses.
2. **Report the file's total size**, in either `Content-Range: bytes 0-0/TOTAL`
   or `Content-Length`. loft discovers the size with a `bytes=0-0` probe before
   its first real read.

A server that **ignores** `Range` and returns `200` with the whole body still
produces **correct answers** — loft reads and discards the prefix to reach the
offset. It is just slow, and slow in a way that grows with the offset. That
fallback exists so a misconfigured host is a performance bug, not a wrong result.

There is also an optional `<url>.dschema` sidecar beside the store (the
self-describing layout descriptor). A **404 is a normal answer** meaning "no
sidecar", not an error.

## Paging, and the knob that matters

Reads are served through a page cache, not issued one-per-lookup:

| | default | env override |
|---|---|---|
| page size | **64 KiB** | `LOFT_PAGE_BYTES` |
| cache size | 64 pages (**4 MiB**) | `LOFT_PAGE_CACHE_BYTES` |

Every fetch is one page-aligned range. **A page is the unit of waste and the unit
of amortisation**: a 12-byte record costs a 64 KiB fetch, and so do the next
thousand records beside it. That is why *what sits next to what* in the file is
the thing to tune — not the page size.

Read this the other way round when you are designing the dataset: records that
are read together should be **written** together. A store whose records are
ordered by the key you query is a store where one page answers many lookups; the
same records in insertion order can cost a fetch each.

## The cost model — round trips, not bytes

The number to think about is **round trips**, and it is easy to get this backwards.
Measured against a live CDN:

```
1 B      41 ms      ← a 64 KiB range costs the SAME as one byte
64 kB    41 ms
512 kB   85 ms
2 MB    221 ms
```

A range request is dominated by its round trip until it gets big. So:

- **Fewer, larger requests win.** Two adjacent pages fetched separately cost
  roughly twice one fetch that covers both.
- **Byte-shaving does not help** while you are latency-bound. Fetching *more*
  bytes in fewer requests is routinely faster.
- **Serial depth is the enemy.** N dependent lookups cost N round trips no matter
  how small each one is. `store_load_keys` exists for exactly this — it opens the
  reader once and reuses its cache, so a batch is not N independent reads.

If a cold read feels slow, count the requests before you count the bytes.

**In the browser the ranges still go one at a time.** Native builds issue a batch's
independent ranges concurrently, so a batch costs about one round trip. A browser has no
threads, and the bytes arrive through a synchronous host bridge, so `--html` spends one
round trip per range — correct, and as fast as it was before batching existed, but the
concurrency win is not there. Budget browser paging by round-trip *count*, and lean harder
on fetching fewer, larger ranges.

**If you run a local Range server for testing, raise its listen backlog.** Concurrent
ranges arrive together, and a small backlog drops the excess SYNs — each dropped one costs
a ~1 s TCP retransmit, so a batch that should take 30 ms takes over a second and reads as a
loft problem. Python's `socketserver` defaults to a backlog of **5**; set
`request_queue_size = 128`. Measured on a real harness: 1066/1285/1075 ms at the default,
27–40 ms at 128.

## Choosing whole vs paged

Paging is not automatically better, and a small file is the case where it loses:

- A dataset of a few MB that is **always needed in full** is one request whole,
  and several requests paged. Use `store_load_url_trusted`.
- A dataset that is large, or needed **sparsely**, is what paging is for.
- The choice is a property of **how you read**, not of how the file was written.
  The same `.store` serves both, so switching is a code change and not a
  re-upload.

That last point is worth leaning on. Because the file never changes, anything
*derived* from it — a prefetch list, a key→page map, an ordering experiment — is
cheap to publish, cheap to revise, and cannot corrupt the dataset. Derived data
over an immutable artifact is the cheap thing to iterate on; the artifact is not.

## Failure modes worth knowing

- **A short read at EOF is not an error.** A range that overhangs the end returns
  what exists; the paged reader tolerates a short tail by contract.
- **An absent key returns `false`**, not a fault. So does an unreadable file and a
  collection that is not the right kind — check the boolean.
- **A truncated or malformed image is REJECTED, never adopted.** Both URL loaders
  validate structure; `store_load_url` additionally pins the SHA-256.
- **`store_verify(local)` proves the load produced a sound heap**, which is worth
  asserting after a paged read from an untrusted origin.
- **Native builds need the bytes-fetching feature** — `remote-store` (the paged
  loaders' own) or `registry`. Without either, a URL is a clear "unavailable"
  error rather than a silent wrong answer.

## Targets

The loft-visible call is identical everywhere; only the byte source differs
(`src/net.rs` is where that difference is confined).

| target | how the bytes arrive |
|---|---|
| native | `ureq`, plus the `file://` offline path |
| browser (`--html`) | the `loft_host_http_range` host import |

The browser case is the interesting one: JS `fetch()` is async and a synchronous
call would deadlock a single-threaded event loop, so the import is on the
wasm-opt `--asyncify` allowlist. Calling it unwinds the wasm stack back to the
event loop, the shell awaits the fetch, then rewinds with the bytes. **The loft
call returns synchronously and the page never freezes** — the program is written
as if the data were local.

## See also

- [STDLIB.md](STDLIB.md) — the `store_load*` family in the stdlib reference.
- [DATABASE.md](DATABASE.md) — stores, and what an image contains.
- [WASM.md](WASM.md) / [BROWSER_INTEROP.md](BROWSER_INTEROP.md) — the browser bridge.
- `src/net.rs` — the per-target fetch; `src/paged_reader.rs` — the page cache.
