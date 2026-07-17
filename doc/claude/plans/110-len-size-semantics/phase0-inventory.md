<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN110 Phase 0 — inventory + baseline

The read-only discovery + visibility baseline for the `len`/`size`-on-`text` flip. This file is
the **worklist** Phase 2 converts against: every site here is a place the flip either moves
behavior (must convert) or is provably safe (no change). See [README.md](README.md) for the
settled semantics.

## The flip, restated (so every classification is unambiguous)

| | today | after flip |
|---|---|---|
| `len(text)` | **byte** count (`OpLengthText` = Rust `.len()`) | **character** count |
| `size(text)` | **character** count (`OpSizeText` = `.chars().count()`) | **byte** count |

So a site's migration is decided by its **intent**:

- **BYTE** — the value is used as a byte offset/bound (slice `s[a..b]`, index `s[i]`, arithmetic
  with `#index`/`#next`/`byte_at`, a scan position). Today it reads `len`. **After the flip it must
  become `size`** or it breaks on any non-ASCII input.
- **COUNT** — the value is used as a human/character count (displayed, "N characters", padding to a
  visible width, column layout, truncation by visible length). Today it reads `len` and is therefore
  **already wrong for non-ASCII** (counts bytes, not characters). **After the flip it stays `len`**
  and becomes correct — the flip *fixes* these.
- **EMPTINESS** — the only use is `len(s) == 0` / `> 0` / `!= 0`. A non/empty test gives the same
  answer in bytes or characters (empty ⟺ 0 bytes ⟺ 0 chars). **Unit-agnostic; SAFE; no change.**
- **SIZE-CHARS** — any `size(text)` call (today = chars). If it wants a char count it must become
  `len`; if it genuinely wants bytes it is already correct after the flip (expected to be rare).

Behavior is identical for pure-ASCII text (bytes == chars); it moves **only** where multi-byte
UTF-8 can appear. The inventory flags multi-byte exposure per site.

## 0a — stdlib (`default/*.loft`) ✅

Every text site lives in `03_text.loft` and `02_files.loft`. **Zero `size(text)` call sites exist
in the stdlib today** — so the `size→len` direction has no stdlib work; all stdlib work is
`len→size` on the BYTE sites below. (`01_code.loft`'s `len(...)` calls are all on `vector`/`sorted`/
`hash`/`spatial` — out of scope; `tree_walk`'s `len(wk_order)` is `vector<T>`.)

### BYTE-intent — convert `.len()` → `.size()` at the flip (Phase 2b)

| file:line | function | site | why BYTE |
|---|---|---|---|
| `03_text.loft:20` | `starts_with_at` | `pl = prefix.len()` | byte length for the slice `self[pos..pos+pl]` |
| `03_text.loft:21` | `starts_with_at` | `sl = self.len()` | byte bound `pos + pl > sl` |
| `02_files.loft:166` | `lines`-splitter (`content` reader) | `if p < c.len()` | byte bound; paired with `ch#index`/`ch#next` |
| `02_files.loft:167` | ″ | `c[p..c.len()]` | byte slice end |
| `02_files.loft:190` | `split` | `self[p..self.len()]` | byte slice end (loop uses `c#index`/`c#next`) |
| `02_files.loft:207` | `split_text` | `n = self.len()` | byte scan bound `i + sl <= n` |
| `02_files.loft:209` | `split_text` | `sl = separator.len()` | byte length for `self[i..i+sl]` |
| `02_files.loft:255` | `valid_path` | `if start < path.len()` | byte bound (`start` from `c#next`) |
| `02_files.loft:256` | `valid_path` | `path[start..path.len()]` | byte slice end |
| `02_files.loft:722` | `dir` | `n = self.len()` | byte index start for `self[i-1]` walk |
| `02_files.loft:737` | `basename` | `n = self.len()` | byte index start for `self[i-1]` walk |
| `02_files.loft:769` | `resolve` | `while t.len() >= 2` | guards byte reads `t[0]`,`t[1]` |
| `02_files.loft:770` | `resolve` | `t = t[2..t.len()]` | byte slice end |
| `02_files.loft:773` | `resolve` | `while t.len() >= 3` | guards byte reads `t[0..2]` |
| `02_files.loft:774` | `resolve` | `bn = base.len()` | byte index start for `base[cut-1]` walk |
| `02_files.loft:790` | `resolve` | `t = t[3..t.len()]` | byte slice end |

16 sites. All are path/scan helpers that in practice see ASCII, so their *observed* behavior rarely
moves — but they are byte-correct only under `size`, so they convert.

### EMPTINESS — unit-agnostic, SAFE, no change

| file:line | function | site |
|---|---|---|
| `03_text.loft:75,87,99,111,123,135,147` | 7 classifiers (`is_lowercase` … `is_control`) | `if self.len() == 0 { return false }` |
| `02_files.loft:189` | `split` | `if self.len() > 0` |
| `02_files.loft:250` | `valid_path` | `else if part != "." && part.len() > 0` |

9 sites. Recommendation: **leave as `len`** — "is the text empty?" reads most naturally as a
logical-count question (`len == 0`), and the answer is unit-independent. (Normalizing to `size` would
be zero-behavior churn; deferred to the 0f coherence decision below — decision: keep `len`.)

**0a totals:** 16 convert (`len→size`), 9 safe, 0 `size(text)` sites, 0 COUNT sites.
(The stdlib has no display/visible-width text code, so no COUNT bugs live here — those are expected
in the renderers/consumers; see 0d.)

## 0b — `lib/*` + registered libraries ✅

All text sites live in **`lib/lexer.loft`** (5 total). Everything else in `lib/` (`code.loft`,
`parser.loft`, `audience_crystal/**`, `docs.loft`, `logger.loft`, `testlib.loft`, `engine_host`) has
zero text sites — the `.len()`/`len(...)` there are all on `vector`/`hash` or are `size`/`length`
struct *fields* (not calls). **Zero `size(text)` calls in all of `lib/`.**

### BYTE-intent — convert `.len()` → `.size()`

| file:line | function | site | why BYTE |
|---|---|---|---|
| `lexer.loft:270` | scan match | `self.data[self.index..self.index + tok.len()] == tok` | byte slice bound into `self.data` — **load-bearing** |
| `lexer.loft:271` | scan advance | `self.index += tok.len()` | advances the byte scan cursor `self.index` |
| `lexer.loft:78` | `set_tokens` | `length: t.len()` in `Possible` | the `length` sort key (`sorted<Possible[-length, token]>`) = longest-match-first; pairs with 270/271 |
| `lexer.loft:80` | `set_tokens` (else) | `length: t.len()` | same sort-key role |

270/271 are the load-bearing byte conversions; 78/80 are convert-for-consistency (a sort key over
ASCII operator tokens — behaviorally inert, but BYTE is the correct bucket).

### EMPTINESS — SAFE

| file:line | function | site |
|---|---|---|
| `lexer.loft:310` | identifier scan | `if result.len() == 0 \|\| self.keywords[result]` |

**0b totals:** 4 convert (`len→size`), 1 safe, 0 `size(text)`, 0 COUNT.

## 0c — `tests/`, `examples/`, `STDLIB.md` ✅

The load-bearing output here is the **ASSERTION canaries** — tests that assert a specific `len`/`size`
*number* on a multi-byte string. These are exactly the sites that go red at Phase 2a; their red set
**is** the flip's cross-check against this inventory. (Text `len`/`size` used as a *bound* inside test
logic, not asserted, only goes red if it produces a wrong multi-byte slice that is then asserted —
which reduces to one of the value canaries below — or it is ASCII and stays green. So the value
canaries are complete for "what the suite shows at the flip"; the full byte-bound conversion is
mechanical, same class as the stdlib byte sites.) `examples/` has no text `len`/`size` sites.

### `len` canaries — expected value changes byte → char at the flip

| file:line | assert (today) | after flip |
|---|---|---|
| `tests/docs/02-text.loft:30` | `len("😃") == 4` | `== 1` |
| `tests/docs/02-text.loft:31` | `len("♥") == 3` | `== 1` |
| `tests/docs/23-safety.loft:106` | `len("Hi 😊!") == 8` | `== 5` |
| `tests/scripts/03-text.loft:10` | `len("😃") == 4` | `== 1` |
| `tests/scripts/03-text.loft:11` | `len("♥") == 3` | `== 1` |
| `tests/scripts/285-json-surrogate-bom.loft:24` | `escaped.len() == 4` (😀) | `== 1` |
| `tests/scripts/285-json-surrogate-bom.loft:32` | `jhi.as_text().len() == 3` (U+FFFD) | `== 1` |
| `tests/scripts/285-json-surrogate-bom.loft:34` | `jlo.as_text().len() == 3` (U+FFFD) | `== 1` |
| `tests/scripts/285-json-surrogate-bom.loft:36` | `jha.as_text().len() == 4` (U+FFFD+'A') | `== 2` |
| `tests/scripts/102-escape-sequences.loft:62` | `s.len() == 26` (`\u{2192}` arrow) | `== 24` |
| `tests/host_input.rs:126` (`.rs` harness) | asserts `"echo=[café] len=5\n"` | `len=4` |

### `size` canaries — expected value changes char → byte at the flip

| file:line | assert (today) | after flip |
|---|---|---|
| `tests/docs/23-safety.loft:107` | `size("Hi 😊!") == 5` | `== 8` |
| `tests/scripts/41-size-text.loft:10` | `size("héllo") == 5` | `== 6` |
| `tests/scripts/41-size-text.loft:11` | `size("日本語") == 3` | `== 9` |
| `tests/scripts/41-size-text.loft:17` | `size("🎉") == 1` | `== 4` |
| `tests/scripts/41-size-text.loft:18` | `size("♥") == 1` | `== 3` |
| `tests/scripts/41-size-text.loft:19` | `size("a😃b") == 3` | `== 6` |

Plus the whole **Section A of the 0e golden fixture** (10 flipping asserts of its 12), designed to
flip. ~90 other numeric `len`/`size` asserts across `tests/` (in `37-stress`, `51-coroutines`,
`85-store-lifetime`, `expressions.rs`, `issues.rs`, `threading_chars.rs`, …) use **pure-ASCII**
strings and stay green — bytes == chars for ASCII. (`str_len` in `n2_cdylib.rs` is a native FFI
helper, not loft `len` — out of scope.)

### Doc-prose canaries — statements of the *old* behavior that must be rewritten in Phase 2d

- `doc/claude/STDLIB.md:130–131` — the table rows (`len` = "Number of bytes", `size` = "code points").
- `default/01_code.loft:677,685` — the stdlib doc-comments on `len`/`size`.
- `tests/docs/02-text.loft:26,29,81` — comments ("len() counts bytes", "long to a human reader",
  "positions are byte offsets, consistent with len()"). After the flip **len IS the human count**;
  these invert.
- `tests/docs/23-safety.loft:100–104` + the assert *messages* on `:106,107` — the len/size explainer.
- `tests/scripts/41-size-text.loft:3` — "size(t) — Unicode character count".
- `tests/scripts/102-escape-sequences.loft:58–60` — "s.len() reports UTF-8 byte length".
- `tests/scripts/105-starts-with-at.loft:5,62`; `tests/docs/00-general.loft:31`;
  `tests/docs/06-function.loft:173`; `tests/scripts/124-vector-amortised-growth.loft:15–18` — comments
  that describe `len` as bytes.
- `doc/claude/STDLIB.md:168` — the identity `c#next == c#index + len(c)` (relies on
  `len(character)`=byte width). Becomes `c#next == c#index + size(c)` under sub-arc 1g — see 0f
  finding 7.

**0c totals:** **17 value canaries** (11 `len` + 6 `size`) + 10 flipping golden-Section-A asserts +
~9 doc-prose blocks. The 17 value canaries (+ golden) are the definitive red set the Phase-2a flip
must reproduce. Non-test high-risk BYTE sites live in `tests/fixtures/libs/` — see 0c-libs below.

### 0c-libs — `tests/fixtures/libs/**` (registered-library test copies, missed by 0b's `lib/`-only scan)

| file:line | site | class | risk |
|---|---|---|---|
| `graphics/src/glb.loft:150` | `json_len = json.len()` → `json_chunk = json_len + pad` | **BYTE — critical** | writes the **GLB binary JSON-chunk byte length**; a non-ASCII glTF mesh/material name → wrong chunk size → **corrupt `.glb` file** after flip |
| `graphics/src/glb.loft:456` | same, scene-GLB path | **BYTE — critical** | same |
| `time/src/time.loft:199` | `cn = len(time_text)` (byte-gate `<5`/`>=8` + `digits_at`) | BYTE | time-string parse over byte offsets |
| `game_protocol/**` + `tests/integration/multiplayer/**` | `raw.len()<5` header gates, `msg[want_prefix.len()..]` slice, displayed `len={got.len()}` | BYTE | binary message/blob byte lengths |
| `web/tests/pack.loft:15,24,34,45` | `s.len() == {2,4,1}` on packed-byte-as-text | ASSERTION/BYTE | byte count of a binary buffer; ASCII bytes today (latent) |
| `arguments`, `web/src`, `moros_editor`, `moros_render`, `imaging` | `len(x) > 0` / `== 0` flag/body/json guards | EMPTINESS | safe |

`graphics/src/glb.loft` is the **2nd-highest-risk site overall** (after `markdown.loft:783`): both write
a byte length into a binary format. Migrate `len→size` (or an explicit byte-length call) with a
non-ASCII regression test.

## 0d — consumers (`moros`, `crawler`, `lib/markdown`) ✅

**Headline: zero COUNT sites across all consumers.** The migration risk here is not "fix a
char-count bug" (there are none) — it is the **29 BYTE sites that must move `len→size`** or they
regress on multi-byte input.

- **`moros`** — **not a loft consumer** (an RPG doc/JS/Python project; 0 `*.loft` files). No sites.
  The plan's consumer list should drop moros or point at the real game(s).
- **`crawler`** (146 `.loft`) — 2 BYTE (`view.loft:146`/`147`, the ellipsis-truncate loop in
  `fit_text`, ASCII-guarded), 3 EMPTINESS, 2 dead "touch-arg" no-ops. **Its real visible-width layout
  (`fit_text`/`wrap_text`) measures width with `gl_measure_text` (pixel-accurate font metrics), never
  `len`** — which is exactly why crawler has no `len`-based char-count bug.
- **`lib/markdown`** — ⚠️ **the canonical `lib/markdown/` is not checked out on this box.** The sweep
  analyzed a faithful mirror at `…/loft2/target/lib-audit/docs/markdown/` (library `markdown` v0.1.0,
  canonical repo `loft-libs-docs`). **Phase 2e must migrate the real repo, not the mirror** — re-run
  the inventory against the checked-out source at convert time. On the mirror: **26 BYTE** + 16
  EMPTINESS in `src/markdown.loft`, +1 BYTE in `tests/01-render.loft`. Zero `size(text)`, zero COUNT.

### Highest-risk consumer BYTE sites (migrate first; would silently corrupt multi-byte output)

| file:line | site | failure if unmigrated |
|---|---|---|
| `markdown.loft:783` | `i = i + char_str.len()` (inline byte cursor) | after flip `len`=1 per char → multi-byte chars under-advance → **cursor desync / garbled inline output** |
| `markdown.loft:567` | `n = src.len()` (master `render_inline` scanner bound) | inline body slices over-run/truncate on multi-byte |
| `markdown.loft:258,974,308,231,222,456` | `ln = line.len(); line[start..ln]` (heading/quote/code/list tail slices over arbitrary text) | **truncates** multi-byte headings/quotes/code blocks |
| URL/`raw` slices `550,556,842,848,836,795,806` | byte slices in `rewrite_image`/`rewrite_link`/`url_part_of` | mangles multi-byte URLs/titles |

The remaining markdown BYTE sites (`384,407,444,472,484,503,520`) and crawler's `146/147` scan
ASCII-structural content (`#`, `=`, `|`, digits) — byte-role but ASCII-safe (behavior-neutral),
migrate for correctness.

**0d totals:** 29 convert (`len→size`), 19 safe (emptiness), 2 dead, **0 COUNT**, 0 `size(text)`.

## 0f — whole-surface unit-consistency audit (no half-features)

The redesign must leave the **entire** text surface coherent, not just `len`/`size`. Every operation
that exposes or consumes a numeric text position must sit unambiguously in one of two worlds, and the
worlds must compose within themselves and never silently across.

### The two worlds, after the flip

**Byte world (O(1), byte offsets) — everything composes under `size`:**

| op | unit | source-of-truth |
|---|---|---|
| `size(text)` | byte count | `OpSizeText` (redefined to `.len()` at flip) |
| `s[p]` | `p` = byte offset; snaps to the char containing byte `p`; out-of-byte-range → null; negatives from end | `text_char_or_raise` + `ops::text_character` (`src/state/mod.rs:3782`) |
| `s[a..b]` | `a`,`b` = byte offsets; `a` snaps back to a char start, `b` snaps forward to a char boundary; out-of-range → empty | `get_text_sub` (`src/state/text.rs:419`) |
| `find` / `rfind` | returns a **byte** offset (`integer?`, null = not found) | native `.find()`/`.rfind()` |
| `byte_at(i)` | `i` = byte offset; returns byte 0–255 (O(1), no UTF-8 decode) | `text_byte_at_native` |
| `c#index` / `c#next` (in `for c in s`) | **byte** position of the char / next char | proven by stdlib usage (`split`, `valid_path`, `lines` pair them with byte slices) |
| `starts_with_at(self, pos, prefix)` | `pos` = byte offset | `03_text.loft:19` (byte-doc'd; internal `.len()`→`.size()` at flip) |

**Character world:**

| op | unit |
|---|---|
| `len(text)` | character count (redefined at flip) |
| `for c in s` | iterates characters (each `c` a `character`) |
| classifiers `is_lowercase`/`is_numeric`/… | per-character predicates over the iteration |

### Surface fully enumerated

The complete `text`/`character` stdlib surface (grep of every `pub fn` on `text`/`character`):
`len`, `size`, `find`, `rfind`, `byte_at`, `starts_with`, `ends_with`, `starts_with_at`,
`contains`, `split`, `split_text`, `trim`/`trim_start`/`trim_end`, `replace`,
`to_lowercase`/`to_uppercase`, `to_text`, `dir`/`basename`/`join`/`resolve` (paths), the 7 `is_*`
classifiers. **There is no `pad`/`repeat`/`center`/`truncate`/`substring`/`char_at`/`char_count`/
`width` helper.** Consequence: the *only* numeric-count producers on text are `len` and `size`;
every position-taking op (`s[p]`, `s[a..b]`, `find`/`rfind`, `byte_at`, `starts_with_at`'s `pos`) is
byte-addressed; everything else returns `text`/`vector<text>`/`boolean` and is unit-neutral. So the
surface has exactly two numeric worlds with no third axis to reconcile.

### Coherence findings

1. **The flip makes each world internally consistent.** After it, the byte world is uniformly
   addressed by `size`: `for i in 0..size(s) { s[i] }` is the correct byte walk, and `find`/
   `byte_at`/`#index` all yield offsets that index `s[...]` correctly. Today `len` is the byte count
   but wears a char-world name — that inversion **is** H2. The flip resolves it; no residual
   byte-world op is left addressed by `len`. ✔

2. **The one cross-world hazard is `for i in 0..len(s) { s[i] }`** — a char count (`len`) driving a
   byte index (`s[i]`). After the flip this over-/under-runs on non-ASCII. It is the *only* place the
   two units can silently collide, and it is defended by the strict-index lint made **default-on for
   text** in Phase 3a (today's opt-in `LOFT_LINT_STRICT_INDEX`). No other op pairs a char count with
   a byte position. ✔ (dependency: Phase 3a must ship with the flip, not after.)

3. **`starts_with_at`'s `pos` is byte-documented** and pairs with `find`/slicing — no signature/unit
   mismatch; only the internal `.len()` migrates. ✔

4. **`find`/`rfind` are byte-documented** and have no char-index counterpart to be confused with, so
   there is no "returns bytes but looks like chars" trap in the API. ✔

5. **No character-indexed slice or "first N characters" primitive exists** — by design (no O(n) char
   random access; characters come from iteration). This is coherent **provided it is documented**:
   "to take the first N *characters*, iterate; `s[0..n]` is bytes." A consumer that today writes
   `s[0..n]` *intending* n characters would have a latent byte/char bug independent of the flip.
   **Resolved by 0b/0c/0d:** no such site exists — every consumer/lib slice is a genuine byte-range
   op (crawler measures visible width with pixel metrics, not slicing; markdown slices byte ranges it
   extracted by byte scanning). The one documentation gap to close in Phase 2 is a one-line note on
   `s[a..b]` / `len` that "characters come from iteration; slicing and `size` are bytes." ✔

6. **Emptiness stays `len`** (finding 0a) — reads as the natural "is it empty?" and is unit-neutral.

7. **`len(character)` — reconsidered in Phase 1 (see phase1-size-impl.md § 1g).** ⚠ The recommendation
   below (`len(character)`=1, `size(character)`=UTF-8 width) was **RETRACTED** once Phase-1 sub-arc 1a
   showed a `character` is a fixed 4-byte slot everywhere it stores inline (`vector<character>`
   strides 4). Corrected recommendation: **`size(character)`=4** (consistent with the scalar/vector
   rule) and **keep `len(character)`=UTF-8 byte width** (the byte-world encoding quantity that
   `#index`/`#next` needs) — do NOT redefine it to 1. Deferred on an owner contract call. The
   original analysis is kept below for the record.

   Today `len(character)` returns
   the character's **UTF-8 byte width** (1–4) — `01_code.loft:693`, `OpLengthCharacter`. After the
   flip, `len(text)` = character count, so `len(character)` returning *bytes* is exactly the
   inconsistency the plan forbids ("`len` means chars for text but bytes for a character"). The
   coherent choice treats a `character` as a 1-character text — **mirror text exactly**:
   - `len(character)` = **1** (one logical unit; or retire the overload), and
   - `size(character)` = its **UTF-8 content byte width (1–4)** — which is *precisely the value
     `len(character)` returns today*, so the quantity is preserved, just renamed to the byte-world
     name. **Caution for Phase 1:** do **not** read the uniform scalar rule to mean
     `size(character) == 4` (its 4-byte storage slot). Just as `size(text)` excludes a text's
     over-allocation and counts *content* bytes, `size(character)` counts the character's UTF-8
     content bytes (1–4), not its fixed 4-byte register width.

   **Blast radius is small but not zero:** no *code* calls `len` on a `character`-typed value (the
   classifiers iterate characters but never call `len(c)`; the `.len()` hits on vars named `c`/`t`
   are all on `text`). But there is **one documented dependency**: `STDLIB.md:168` states the
   identity `c#next == c#index + len(c)` (byte offset of the next char = current + the char's byte
   width) — this relies on `len(character)`=bytes. Under the fix that identity becomes
   `c#next == c#index + size(c)` (and must be updated in the doc). **Recommendation (sub-arc 1g):**
   add `size(character)` = UTF-8 content byte width (1–4) and redefine `len(character)` = 1 (or drop
   the overload); update the `#index`/`#next` identity to `size(c)`; document both. So `len`/`size`
   mean the same thing on `text` and `character`. *(This is the one real half-feature the audit
   surfaced — everything else in the surface was already coherent.)*

**0f status:** ✅ surface map complete and coherent on the numeric axis; the watch item (finding 5) is
resolved — no char-intended slicing anywhere. No half-feature found. The only Phase-2 surface action
is a doc note (finding 5) + the strict-index lint default-on (finding 2, Phase 3a).

## 0e — golden-behavior corpus

_See `tests/scripts/pln110-text-surface-golden.loft` (landed additive/green) — pins today's
pre-flip numeric behavior of the whole surface across ASCII / 2-byte / 3-byte / 4-byte / mixed
inputs, so the Phase-2 flip produces a reviewed diff, never a silent move._
