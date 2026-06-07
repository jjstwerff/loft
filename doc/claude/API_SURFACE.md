<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# API_SURFACE.md — verifying the two prime programmer-facing surfaces

A programmer touches loft through exactly two API surfaces:

1. **the language + stdlib** — syntax plus the functions every program calls
   (`default/*.loft`), and
2. **the libraries** — graphics, shapes, web, … (the registry packages).

Both are *read and called by humans*. Both are where a confusing name, an
accidental duplicate, or an undocumented function becomes a user's problem. So
both must be **verified for API quality** — and with one tool, because:

> **The stdlib is the library every program imports.** It passes
> [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) like any other library, and the
> audit below is one auditor run over **two targets**: `default/*.loft` and each
> library's `pub` surface.

This is [Goal F](GOALS.md) (friction-free) and [Goal B](GOALS.md) (legibility)
applied at the **named-API** level: F guards the *syntax*, this guards the
*vocabulary* a programmer learns and types.

---

## Goals

| | Goal | Why it bites the user |
|---|---|---|
| **S1** | **No accidental duplication; safe default** | Two functions doing one job → "which do I call?"; they drift apart and one rots. And when variants *do* exist, the shortest/default name must be the **safe** one — users reach for the short name (C's `gets`/`strcpy`/`atoi` were lethal precisely because the convenient name was the unsafe one). |
| **S2** | **No confusable names** | `len`/`length`, `to_text`/`to_string` a keystroke apart → silent wrong call, or constant doc-lookup. |
| **S3** | **Everything documented** | An undocumented `pub fn` is a guess; the programmer reads the source or trial-and-errors. |
| **S4** | **No footguns** | Two adjacent params of the same type silently swap (the textbook case: C's `memset(s, c, n)` — value and count both `int`); a partial function with no bounding error returns garbage. |
| **S5** | **Names express intent, consistently; no implementation leak** | One spelling per concept (`_to_text` everywhere, never sometimes `_to_string`); the name predicts the behaviour (not C's `strcmp()==0`-means-equal). And an **implementation constraint must never name the API** — C's `creat` lost its `e` to a 6-char linker limit, permanently. |
| **S6** | **No brittle setup, no hidden state** | A required `init()` before `use()`, a global one call sets and another reads, a constructor that returns a half-built-but-usable object — the user must reconstruct an unseen setup contract, and getting it wrong fails *silently*. |

**S6 is the project's brittleness doctrine, with the user as the second deriver.**
[GOALS.md § "Don't tolerate re-derivation patterns"](GOALS.md) defines brittleness
as *hidden, re-derived shared state that fails silently*. At the API surface the
re-deriver is **the programmer**: a setup-order contract or shared mutable state
that lives only in the library author's head forces every caller to rebuild it, and
the symptom of getting it wrong is a wrong result, not an error. The cure is the
same constructive law — **remove the invariant or make its violation loud**:
- **encode it in the types** — `use()` takes the handle `init()` returns, so the wrong order won't compile; or
- **eliminate the hidden state** — each call is self-contained, no global to set first; or
- **make violation loud** — calling out of order is a clear bounding error ("not initialized — call `init()` first"), never silent-wrong.

A library passes S6 when there is **no usage sequence whose violation is silent**.

**The overload caveat (load-bearing).** Raw name-collision is *not* the signal —
a scan of today's stdlib finds 32 names defined more than once, and **nearly all
are legitimate overloads** (`to_text` across 7 types, `len` across 6, `abs`/`min`/
`max`/`clamp` over int+float, trig over int+float). The auditor must distinguish:

- **legit overload** — same name, *different* type signatures, *same* concept — **keep**;
- **accidental dup** — same name **and** same signature — **flag**;
- **asymmetric overload set** — a concept defined on 7 of 8 types a user expects → a *gap*, not a dup — **flag**.

A dumb name-counter would file 32 false positives and hide the real one. The
detector keys on **(name, signature) + concept coverage**, not name alone.

---

## What we verify — `[auto]` vs `[review]`

Same two-tier split as [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md): machines
enforce what fails silently, humans judge what needs taste.

| Check | Goal | Tier |
|---|---|---|
| Exact duplicate (same name **and** signature) | S1 | `[auto]` |
| Asymmetric overload set (concept missing on a sibling type) | S1 | `[auto]` |
| Confusable-name cluster (normalized-collision / small edit distance) | S2 | `[auto]` |
| Missing doc comment on a `pub fn` / `pub struct` | S3 | `[auto]` |
| Naming-convention drift (`t_<LEN><Type>_`/`n_` rules; one spelling per concept) | S5 | `[auto]` |
| Same-type adjacent params (swap hazard) | S4 | `[auto]` (heuristic) |
| Mutable module/global state in a library; a `pub fn` reading state another `pub fn` must set | S6 | `[auto]` (hint) |
| Every example/test opens with the same setup call (a setup phase the API should fold in or enforce) | S6 | `[auto]` (hint) |
| Redundant / overlapping functions (two names, one job) | S1 | `[review]` (with `[auto]` hints: same return + overlapping args) |
| Footgun semantics (partial fn w/o bounding error, silent-wrong) | S4 | `[review]` |
| Setup-order dependency: encoded in types, or merely documented? Omission loud or silent? | S6 | `[review]` |
| Does the name + doc actually express intent | S3/S5 | `[review]` |

**Baseline reality** (stdlib scan, 2026-06-07): **44 of 146** user-facing
functions have no doc comment; 0 confusable-by-underscore clusters; 32
name-collisions all-but-certainly legit overloads pending the signature refinement.
The 44 are the first concrete `[auto]` worklist.

---

## Prior art: the C standard library

C is the most-used API in history, designed (1970s) before most of these lessons
existed — so it is the empirical proof that S1–S6 are real. **Nearly every famous
C-stdlib footgun maps to one Sn violation, and C's own retrofitted fixes are the Sn
cures, paid for 25–40 years late.**

| Goal | C failure | What it teaches |
|---|---|---|
| S1 | `atoi`/`strtol`/`sscanf` (3 int-parsers); `gets`/`fgets`; `strcpy`/`strncpy` | The dangerous one is always the **short, convenient** name — users reach for `gets`/`atoi`/`strcpy` *because* they're shortest. ⇒ the default name must be the safe variant. |
| S2 | `strchr`/`strrchr`, `strspn`/`strcspn`/`strpbrk`, `scanf`/`sscanf`/`fscanf` | Cryptic abbreviations from the **6-char linker-symbol limit** — a toolchain constraint that leaked into the permanent user vocabulary. |
| S3 | functions are documented, but the hazard (static-buffer returns, `errno`, thread-safety) is buried in prose | Doc *presence* ≠ surfacing what goes wrong. |
| S4 | `gets` (overflow by design); `strncpy` (the "safe" one skips null-termination on truncation); **`memset(s, c, n)`** (value + count both `int` → the canonical swap bug); `printf(user_str)` format-string vuln | `memset` is a literal instance of our same-type-adjacent-param `[auto]` check — proof the heuristic catches real bugs. |
| S5 | `strcmp` returns **0 on equal** (0 = success collides with 0 = false); `creat` (the missing `e`); inconsistent return conventions | The name must predict behaviour; an implementation limit must never name it. |
| S6 | `errno` (global, silent); `strtok` (hidden static cursor — nested/concurrent use silently corrupts); `localtime`/`asctime` (return a static buffer overwritten next call); **`setlocale` silently changes how `strtod` parses `"3.14"`** | The worst offenders — and the clearest cure (below). |

**The clever engineering is the *same move* as the cure.** C's best work is exactly
what S6 prescribes — which is why the goals push toward good engineering, not away:

- **`FILE*`** is an opaque handle that **reifies the stream state you pass explicitly** — precisely S6's "encode the state in a handle, don't hide it in a global." stdio got S6 *right* while `errno`/`strtok` got it *wrong*, in the same library.
- **The `_r` variants** (`strtok_r`, `localtime_r`) are C *confessing* S6: the fix was always "take the state as a parameter." The `_r` suffix **is** the doctrine, retrofitted.
- **Thread-local `errno`**, **`calloc`'s `count × size` overflow check**, **`qsort`/`bsearch`** (generic algorithms with no generics), and **freestanding minimalism** are all real virtues our goals must not trample.

**The irreversibility argument — why the gate must be pre-publication.** C carried
`gets` from 1978 to its C11 removal in **2011**. You cannot un-ship an API; the only
affordable time to catch an Sn violation is *before* the first release. That is the
entire reason `api-lint` runs as a gate and the registry `verified` mark is withheld
until a library passes — the same logic [Goal F](GOALS.md) uses for syntax (once
ceremony ships, it can't be removed).

## What we need to implement

1. **Enumerator** — list a target's public API (name, signature, doc-comment
   presence, defining type). `gendoc` already walks the whole API for HTML
   generation; reuse that pass rather than re-parsing. Target = `default/*.loft`
   **or** a library's `pub` surface (same code, different root).
2. **`api-lint` — the `[auto]` checks** (`scripts/api_lint`, later `loft api-lint`):
   exact dup, asymmetric overload set, confusable cluster, missing doc, naming
   drift, same-type adjacent params. Emits a structured report (path, item, check,
   severity).
3. **Baseline triage** — run over the stdlib first; **fix-first or allowlist** the
   existing 44 undocumented + any asymmetric sets *before* the gate goes red, so CI
   doesn't break on day one (the reify → prove-on-real-data → cut-over arc).
4. **Wiring** — `[auto]` gate runs in `make ci` (stdlib) and in `library-ci.yml`
   (libraries — the same gate [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) already
   names). `[review]` items fold into the existing human gates: the loft PR review
   (stdlib) and the registry PR (libraries).
5. **One shared standard, two surfaces** — `LIBRARY_CHECKLIST.md`'s API-quality
   row points here; the language docs point here; the text lives **once**.

### Phasing

| Phase | Deliverable | Catches |
|---|---|---|
| **1** | Enumerator + missing-doc + exact-dup checks, run over the stdlib | the 44 undocumented; any true dup |
| **2** | Confusable cluster + asymmetric overload + naming drift | S2/S5 + the real S1 signal |
| **3** | Wire `[auto]` into `make ci` + `library-ci.yml` (after baseline clean) | the standing gate, both surfaces |
| **4** | Footgun heuristics (`[review]` hints) | S4 |

---

## See also
- [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) — the full correct-library bar; its API-quality items are this audit applied to a library.
- [GOALS.md](GOALS.md) — Goal F (friction-free) + Goal B (legibility), which this serves at the named-API level.
- [DOC_QUALITY.md](DOC_QUALITY.md) — the standard a doc comment must meet (S3): present-tense why-to-use, plain language.
- [CODE.md](CODE.md) — the naming conventions S5 enforces (`n_`/`t_<LEN><Type>_`).
- [DOC.md](DOC.md) — `gendoc`, whose API walk the enumerator reuses.
