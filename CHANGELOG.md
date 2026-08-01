---
render_with_liquid: false
---
# What's new in loft

A short, friendly log of what has changed in each release.  Read top-to-bottom
for a tour of how the language has grown.

Looking for the deep technical history (opcode renames, slot allocator
invariants, internal phase numbers)?  See
[doc/claude/CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md).

---

## 2026-08

The **heap-correctness** release. Almost everything here is one theme seen from a
dozen angles: a value that outlived the storage it pointed into, or storage that
outlived the value. Closures that build structs, elements taken out of what a
function just returned, keyed collections that drop entries, captures that changed
meaning with declaration order — all of them read correctly once and wrongly later,
which is the shape that cannot be found by reading the code. They are fixed, and the
detectors that would have caught them earlier are now part of the nightly gate.

Alongside that: a store can give its file back (`store_reclaim`, plus automatic
compaction at load), `reserve(v, n)` for vectors you know the size of, a crash report
that survives being piped somewhere, and `u32` finally holding every `u32`.

### An element taken out of a returned value stays valid

Binding an element out of a struct a function just returned gave you a reference
into storage that had already been handed back:

```loft
plan = roof_plans().items[0] ?? RoofPlan {};
```

It read correctly at first and turned to zeroes once other allocations reused
that memory. In the program that reported it, the same binding answered three
questions correctly and the fourth wrongly, in one function with nothing between
them but ordinary library calls — a test asserted a roof's ridge height and got
its eave's. A value that is right the first time and wrong the fourth cannot be
found by reading the code, which is the worst shape this kind of bug takes.

The same expression **inside a loop** was a second, separate fault with the same
cause and a different trigger. There the damage depended on what else the loop
did: with an allocation between the bind and the read, the element was read out
of memory that had already been reused, so

```loft
for i in 0..50 {
  e = make().items[0] ?? P {};
  o = other(i);              // claims the memory `e` points into
  sum += e.a;                // read 0, 1, 2 … instead of e's real value
}
```

summed the loop counter instead of the element. Without such an allocation it
returned the right answer while still reading freed memory, so it was invisible
until the arena poison detector was pointed at it.

Both are fixed. The workaround the issues documented — binding the returned value
to a local first, then indexing it — is no longer needed:

```loft
held = roof_plans();                          // no longer necessary
plan = held.items[0] ?? RoofPlan {};
```

### A closure that builds a struct no longer crashes

A closure that captured something and called a function returning a struct
crashed the interpreter outright:

```loft
k = 4.0;
pt = fn(i: integer) -> Point { make_point(k) };
p = pt(0);                       // SIGSEGV
```

Worse, the compiled build got it right, so the same program behaved differently
depending on how you ran it. Both halves were needed to trigger it — a closure
that captures nothing was fine, and so was one that builds the struct inline
instead of calling for it — which is why it survived so long and then hit a real
program doing something perfectly ordinary.

It is fixed, and the two backends agree again.

Using such a result *inline* also leaked one record per call — the buffer holding
the returned struct had no variable to hang its free on, so nothing released it.
That is fixed as well, and neither form leaks now:

```loft
total += pt(r).a;            // no longer leaks
p = pt(r); total += p.a;     // and neither does binding it first
```

### Removing from a keyed collection, without the crashes

Three ways of removing from a keyed collection went wrong, all now fixed:

- Removing entries from an `index<T[..]>` whose records own a `text` could
  crash while the loop was still running.
- Simply *declaring* a struct with both a `sorted<T[..]>` and an `index<T[..]>`
  over the same element type was enough to break that type everywhere — the
  interpreter hung and the compiled build produced wrong code. No removal
  needed; the declaration did it.
- Removing by key from a `spatial` collection is covered in its own entry below.

### `spatial` collections answer to a point

A `spatial<Mob[x, y]>` could be appended to, iterated, counted and range-sliced,
but the plain point subscript did not work — in three different ways, which is
why it read as one small bug:

```loft
mobs: spatial<Mob[x, y]> = [];
m = mobs[3, 6];                          // crashed
mobs[3, 6] = Mob { x: 3, y: 6, hp: 10 }; // could destroy the collection
mobs[3, 6] = null;                       // corrupted the store
```

Reading a point crashed the interpreter with an index-out-of-bounds while the
compiled backend answered correctly, so the same program behaved differently
depending on how you ran it. Assigning at a point that held nothing did not
insert — it wrote over the collection itself, and four elements read back as
one. Removing was the case that got reported, and it either corrupted the store
or refused to compile.

All three now work, and behave as they do on a `hash`: `xs[x, y]` reads (`null`
when the point is empty), `xs[x, y] = value` inserts or replaces, and
`xs[x, y] = null` removes. Note the coordinates are separate subscripts here —
`xs[3, 6]` — where the range forms parenthesise them, `xs[(3,6)..(9,9)]`.

### A crash report you can still read afterwards

When loft dies of a segfault it prints what it was doing: the last opcode, where
it was in the program, and which function. That went to stderr only — so a build
that filters stderr threw it away, and the one run that could explain the crash
was also the one run you cannot repeat.

The report is now written to a file as well: `.loft/loft-crash-<pid>.txt` next to
the package, or your temp directory when there is no `.loft/`. The report on
stderr names the file it wrote. Set `LOFT_CRASH_FILE` to put it somewhere
specific, or set it to empty for the old stderr-only behaviour. A run that does
not crash writes nothing.

### Removing from a collection now gives the memory back

Removing an element unlinked it and stopped there. The element's storage — and
anything it owned, a `text` or a nested `vector` — was never released, so a
program that keeps a collection at a steady size still grew forever:

```
cycle 0: 300 records live, 0.10 MB claimed
cycle 5: 300 records live, 0.56 MB claimed     // same 300 records
```

Emptying a collection and refilling it grew the store rather than reusing the
space. Every collection kind was affected and every element shape; records
holding only numbers leaked least, which is why this could sit unnoticed.

Now `c[key] = null` and `e#remove` release what the element owned, the space
returns to the free list, and refilling reuses it — a long-running program that
adds and removes stays flat instead of climbing. Nothing to change in your code.

If you were working around it by reusing element objects instead of removing
them, you can stop.

### `reserve(v, n)`, for when many vectors grow at once

Appending to a vector doubles it when it runs out, which is the right default and
almost never something you think about. It becomes something you think about when
*many* vectors grow together — a generator reading a stream and appending each
item to whichever collection owns it. Every vector is then sitting immediately
before another one, so no growth can extend in place, and each one copies itself
to a new block roughly log N times on the way up.

```loft
for tile in tiles { reserve(tile.points, expected_count(tile)); }
for feature in stream { tiles[feature.tile].points += [feature.point]; }
```

`reserve` only ever changes how much room a vector has. It cannot change its
length, its contents, or anything holding a reference to it, and asking for less
room than it already has does nothing.

The saving shows up on disk too, because a persisted store carries each vector's
claimed capacity rather than its length: on a 125-record × 2312-coordinate store,
reserving took the file from 3,816,152 to 2,609,736 bytes for byte-identical
data.

### `directory("sub")` now appends the subpath it always advertised

`directory`, `user_directory` and `program_directory` each take an optional
subpath, and each quietly ignored it:

```
directory("sub")            // was: /home/you/project      now: /home/you/project/sub
program_directory("assets") // was: /usr/local/bin/loft    now: /usr/local/bin/assets
```

The argument doubles as the buffer the answer comes back in, and it was cleared
before being read. Nothing reported this — you got a valid directory, just not
the one you asked for.

`program_directory()` also returns what its name says now: the directory
**containing** the executable, not the executable's own path. Appending to the
old value produced `/usr/local/bin/loft/assets`, a path that can never exist,
and the browser build already answered with a directory — so the two targets
disagreed. If you were compensating for this by trimming the binary name
yourself, drop that step.

### A file outside your project is now equally out of reach for writing

`file("../notes.txt")` reported the file absent, as paths outside the project
always have — but `f.write(…)`, `f += …` and `write_bytes(…)` went ahead and
created it. The documented way to append,

```
f#next = f.size;   // seek to the end
f += "more";
```

then read `f.size` as 0 and overwrote the file from the start, silently. Reads
and writes now agree: a path outside the project cannot be written either, and
the attempt is visible — `f.write(s).ok()` is `false`, `write_bytes` returns
`false`. Move the file inside your project, or use an absolute path (an
absolute path is not restricted; build one with `directory()` or
`user_directory()`).

### A browser build no longer refuses calls it cannot serve

Naming a builtin the browser has no handler for — `gl_screenshot`, say — failed
the `--html` build outright, so one source had to become two entry points
differing only in which calls they were allowed to mention. It builds now: the
call returns its usual failure value (`false`) and reports itself once in the
browser console, which is what a caller checking the result already handles. The
build still tells you which calls those are.

### Declaration order no longer changes what a closure sees

Loft lets you use a struct before you declare it, and that is meant to be invisible.
Inside a lambda it was not:

```
fn take(w: World) -> float {
  chunk = w.chunks[1];                                   // World declared further down
  f = fn(x: float) -> float { chunk.cells[2].v * x };
  f(2.0)
}
```

That failed with `Unknown field text.cells` — a complaint about `text`, in a program with
no `text` anywhere. Moving the `struct World` above the function fixed it, which is a
confusing thing to have to discover.

The capture now resolves exactly as it does when the declaration comes first, whatever it
was projected out of — a field, a vector element, a whole vector, or a scalar read out of
one. Declaration order is invisible again.

### The last corner of mutable text captures

Making mutated parameter captures work (below) left exactly one combination refused with
a message: mutating a captured `text` **parameter** inside a function that itself returns
`text`. That works now too, so the rule for closures is finally uniform — a mutated
capture behaves the same whether it started as a local or a parameter, and whatever the
enclosing function returns.

What was behind it: a text value the function *returns* is already handled specially (the
caller supplies the buffer), and the compiler had been keying off "does this function
return text?" rather than "is this the value being returned?". The first question is the
wrong one — a function can return one text while a closure mutates another, and only the
second question tells them apart.

### A closure can now count with one of your parameters

The accumulator closure is an old friend:

```
total = 0;
add = fn(n: integer) { total = total + n; };
```

It worked over a local, and crashed the moment `total` was a **parameter** of the
enclosing function — with a garbage function id, a segfault, or, on the compiled
backend, generated code that would not build. Every scalar type was affected, and you
did not even have to call the closure: creating it was enough.

Now a mutated parameter behaves like a mutated local, which is what it looks like. The
closure's writes are visible for the rest of the function, and your **caller's value is
untouched** — a scalar parameter is still passed by value. Mutating a `const` parameter
through a closure is refused, as it is anywhere else.

### A lambda that captures your value no longer eats it

Handing a lambda a value from the surrounding scope is the ordinary way to give a
library a function it can call:

```
sampler = fn(x: float, z: float) -> float { terrain_y(x, z, world) };
rest = ground_axle(sampler, cx, cz, yaw, half, radius);
```

If `world` was a **parameter** of the enclosing function, that closure destroyed the
caller's `world`. Nothing said so at the time — the value stayed readable for a
while — and the failure landed later, in some unrelated function that happened to
touch the same value next, with a crash pointing hundreds of lines away from the
lambda. Capturing a value that only *views* into another (`chunk = world.chunks[1]`,
or a `for` loop's element) did the same thing.

The rule is now the one you would expect, and it needs nothing from you: capturing a
value the function **owns** hands it to the closure, which is what lets a factory
return a closure over its own local; capturing a **parameter** or a view **borrows**,
because the real owner outlives the closure either way. So passing a value into a
function that captures it in a lambda cannot damage your copy, and a returned closure
never reads something already freed.

Nothing you can write got stricter — code that avoided closures over store-backed
values by hand keeps working, and can now stop avoiding them.

### Changing a value you matched on now sticks

Destructuring in a `match` gives you a **view** of what you matched, so writing
through it changes the value:

```
match e {
    Holder { items } => { items += "y"; }     // `e`'s payload really is longer now
    _ => { }
}
```

That is what it always looked like it did, and for a vector payload it is what it did.
For a `text` payload the write was silently thrown away — same syntax, same shape, no
warning, and nothing in the code to tell you which one you had written. It now behaves
the same whatever the payload type, including through nested patterns like
`Wrap { inner: Holder { items } }`, where the write used to vanish for every type.

Two crashes in the same corner are gone with it. A function that returns an enum,
calls itself, and edits the payload before returning it used to segfault the
interpreter while the compiled backend quietly got the right answer — a
particularly unpleasant pair, because the browser runs the interpreter. And three
enums in one file that happen to share a variant name (`Nil`, say) used to abort
the compiler with an internal error, on perfectly ordinary code.

If you had worked around any of these by rebuilding the value instead of editing it
in place, that code keeps working — nothing you can write got slower or stricter.

### A big function no longer breaks its own loops

A function whose body grew past about 32 KB started behaving impossibly: a
`while true` would run its body **once** and then simply carry on past the loop,
and the program would exit reporting success. No error, no warning, no hang. Every
`println` inside the body still ran exactly once, so a log ended mid-story with
nothing wrong in it — there was nothing to follow.

The cause was that the interpreter recorded "how far to jump" in a 16-bit number.
Past 32 KB that number no longer reached, and the jump landed somewhere arbitrary.
It affected **every** kind of jump — `while`, `for`, `break`, and skipping over an
`if` or `else` body — because they all recorded the distance the same way. The
compiled backend (`--native`) was never affected.

Jump distances are now 32-bit, which covers any program loft can hold, so this
cannot come back at a larger size. Nothing to change in your code: functions that
were too big simply work now. (Reported from moros, whose editor server sat right
on the edge — adding two `println` lines made it exit before any client could
connect.)

### A `vector<vector<u8>>` finally reads back what you put in

Nesting a **narrow** number inside a vector — `vector<vector<u8>>`, `<u16>`, `<i16>`,
`<i32>`, `<u32>` — used to be misread the moment you did anything with the whole
collection. Printing one packed two 2-byte values into a single 8-byte slot
(`[[1001,2002],…]` came back as `[[131204073,0],…]`); with a 1-byte inner it crashed
outright. Slicing gave the right number of rows with the contents emptied. Reading one
element at a time was always correct, which is what made this so easy to miss: a length
check passes, a spot-check of `v[0][0]` passes, and only a full comparison shows the
damage. If your byte vectors carry real data — a file, a hash, a ciphertext — that is a
silent corruption, and it is what the consumer who reported this was carrying.

The cause was that loft's type table could not actually *say* "vector of vector of
`u16`": a nested element collapsed to its inner scalar, so a declared
`vector<vector<u16>>` registered as `vector<vector<integer>>` and nothing downstream
could know the real width. Seven different places had each worked out the element's
storage width for themselves, and they agreed only when that element happened to be 8
bytes wide — exactly the boundary where the bug started. They now all read one answer,
so a reader can no longer stride differently from the writer. Nesting depth 3+, every
construction path (literal, typed local, `+=`, function return), slicing, printing,
concatenating and binding into a struct field are all covered by the new guard.

Two related fixes came with it. `((v[i] ?? 0) & 255) as u8` — pulling a byte out of a
`vector<u8>`, masking it, and casting — now compiles on `--native`; it used to fail with
a raw `rustc` type error unless you bound the value to a local first, and that workaround
can go. And reading an **unsigned** narrow integer from a binary file on `--native` now
zero-extends: a `u16` of `0xBEEF` read back as `-16657` in some contexts.

One deliberate change comes with this: `size(v)` on a nested vector now reports each row
as its 4-byte reference rather than the inner scalar's width — so
`size(vector<vector<integer>>)` with two rows is 8, not 16. That is what `size`'s
contract already promised ("a heap element counts as its record pointer, never the
target's content"); the number had been following the stride bug. `len` is unaffected,
and no on-disk layout changes.

### `u32` finally holds every `u32`

Three fixes to narrow-width integers, all found by the crawler consumer building a
collision grid, all affecting the interpreter and `--native` alike.

**`x as u32?` used to return null for every value.** The checked-cast range guard was
built at 32-bit width, and `u32`'s maximum wraps there — so the test became "is the value
at most -2?", which nothing satisfies. This was silent rather than loud: the cast handed
back null, the idiomatic `?? 0` supplied a plausible-looking zero, and a grid of zeroes
reads as *not filled in yet* rather than *corrupt*. A constant like `70000 as u32?` worked,
which is why it only ever showed up through a helper function.

**`v[i] = x` now works on a `vector<u16>` and a `vector<i16>`.** It used to fail to
compile outright ("Cannot assign to attribute on type 'OpGetShortRaw'"), even though the
same write to a `u16` *struct field* was fine.

**A `u32` above 2 147 483 647 now round-trips.** Storage for 4-byte integers was
signed-only, so `4000000000` read back as `-294967296` — in vectors and in struct fields.
Worse, `2147483648` collided with the internal null marker and read back as `null`. Both
are fixed: `u32` now covers its full documented range, and the reserved code sits at the
top (`4294967295`), where no legal `u32` value can reach it. The stored bytes are a plain
native `u32`, so binary formats see exactly what they expect. `i32` is untouched.

If you worked around any of this by using `i32` where you wanted `u32`, or `integer` where
you wanted `u16`, you can now use the narrow type — and get the smaller footprint with it.

## 2026-07

The **stability and type-safety** release. Two things anchor it: parsing text into a
number is now *honestly fallible* (it hands you a nullable instead of quietly
inventing a `0`), and a long-standing class of heap / memory bugs — leaks and
use-after-free corruption around returns, reassignment, and `match` — has been
retired wholesale and is now guarded on every night's CI. The registry, the sandbox,
and reference binding all move forward too.

### Patch `2026.7.2` — the `len`/`size` text flip (breaking, pre-1)

**Breaking (contract 0 — before the 1.0 promise):** `len(text)` now returns the
**character** count and `size(text)` the **byte** count — swapped from earlier
`2026.7.x`, where `len(text)` was bytes. This aligns `len` with every mainstream
language (a text's length is how many characters it holds) and gives byte-level work
an explicit, honest home in `size`. A program that used `len(text)` to mean a **byte**
length must move those sites to `size(text)`; a default-on lint flags the common
`for i in 0..len(s) { s[i] }` shape, where a character count was driving a byte index.

Because this changes observable behaviour, published libraries that depend on the new
meaning now declare `loft = ">=2026.7.2"`, so an **older** loft cleanly refuses to load
a too-new library at load time rather than silently misreading multi-byte text.

This point release also carries the accumulated stability and type-safety work landed
since `2026.7.1`: the compatibility-contract groundwork (@PLN102), PEG `match` patterns
(@PLN35), the state-of-the-distribution catalogue (@PLN112), layout-aware zero-copy FFI
(@PLN105), and a wide store-lifetime / leak / use-after-free sweep now guarded on every
night's CI. Full technical history:
[doc/claude/CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md).

### Patch `2026.7.1` — the downloadable binaries

`2026.7.0` shipped without its pre-built binaries: a release-pipeline bug published
the release before the platform builds could attach, and the published release was
immutable. `2026.7.1` is that same release with the four platform bundles actually
attached — Linux (`x86_64-musl`), macOS (Intel + Apple Silicon), and Windows — plus
the pipeline fix (binaries are now built into a draft *before* publish) so it cannot
recur. The loft binary itself is unchanged from `2026.7.0`.

### Parsing a number can fail — `text as integer` now gives you a nullable

This is the one change most existing code will need to look at.

- `"42" as integer` used to *always* hand back an integer, silently producing `0`
  when the text wasn't actually a number — a wrong answer that looked like a real
  one. Casting text to a number is now an honest **fallible parse**: `text as
  integer`, `text as float`, and `text as single` return a **nullable** (`integer?`,
  `float?`, `single?`), and yield `null` when the text isn't a valid number.
- `"42" as integer` is `42`; `"oops" as integer` is `null`.
- Handle the `null` at the cast site, most often with `??`:
  `count = field as integer ?? 0`, or keep the `integer?` and test it
  (`n = field as integer; if n { … }`).

> **Upgrading:** anywhere you wrote `x = some_text as integer` (or `as float` /
> `as single`) and then used `x` as a plain number, add a default —
> `x = some_text as integer ?? 0` — or give `x` a nullable type. The parse silently
> returning `0` on bad input is exactly the bug this closes, so the compiler now
> makes you say what should happen. `null as integer?` is how you write an explicit
> typed null.

### Float math that can't answer now hands back a nullable

The same honesty reaches floating-point arithmetic. A handful of operations have no
real answer for some inputs — `/` and `%` when the divisor is `0`, and `sqrt`, `ln`,
`log2`, `log10`, `asin`, `acos`, and `pow` outside their domain. Those now produce a
**nullable** (`float?` / `single?`) instead of a silent `NaN`, and the nullability
**propagates**: a value computed from a `float?` stays `float?` until you settle it.

- `sqrt(x)` and `a / b` are `float?`. Storing one into a spot that must hold a real
  number — a `-> float` return, a `not null` field — is where you discharge it:
  `sqrt(dx*dx + dy*dy) ?? 0.0`, or keep the `float?` and test it.
- A **literal operand known to be in range keeps the result non-null** — `x / 2.0`,
  `pow(x, 2.0)`, and `sqrt(2.0)` stay plain `float`, so most everyday arithmetic is
  untouched. `+`, `-`, and `*` on non-null floats are always `float`.
- This is a **warning, not an error**. Existing programs keep compiling and keep
  their old runtime behaviour; the compiler just marks each place a possibly-null
  float lands somewhere that expects a definite number.

> **Upgrading:** if a `-> float` function ends in a variable division or a
> `sqrt`/`pow`/`ln`/… of a variable, add a default at the boundary — `… ?? 0.0` — or
> widen the type to `float?`.

### A written compatibility contract — and a command to check it

loft now states its **compatibility contract** in writing
([COMPATIBILITY.md](doc/claude/COMPATIBILITY.md)): at contract level 1, a program
that runs today keeps running — the language, its error behaviour, and its libraries
all included. Both changes above fit inside it, which is exactly why the
nullable-parse and float-null shifts surface as **warnings** rather than breakage.

A new `loft api-surface` command makes the contract checkable. `loft api-surface
<file>` prints the public surface of a program or library — its `pub` functions,
types, sizes, and signatures — and `loft api-surface --diff <base> <new>` compares
two versions and reports a plain verdict (drop-in, or a break), exiting non-zero on a
break so a CI check can guard it.

### Error messages now point at the problem

Every error — parser, type, or runtime — now shows the file, line, and column,
the offending source line, and a caret under the exact token:

```
error: expected integer, got text on argument 2 of call to add
  --> game.loft:3:14
  |
3 |   x = add(1, "two");
  |              ^
```

- **Did-you-mean suggestions.** A misspelled variable, function, field, method,
  type, or enum variant suggests the near match — `unknown variant Color::Bleu —
  did you mean 'Blue'?`.
- **Concrete type mismatches** name both sides, the operation, and (for calls)
  the argument index — no more bare "type mismatch".
- **A mistyped `match` pattern** that can never match its subject (a text arm on
  an integer subject) is now a clear error instead of a silently-dead arm.
- Prefer the old single-line format? Set `LOFT_ERRORS=compact` (or pass
  `--errors=compact`).

Runtime faults (divide-by-zero, index out of bounds, null dereference,
narrowing-cast overflow) keep loft's rule of never aborting a running program:
they yield loft's usual sentinel (`null`, `0`, …) so a game or server keeps
running, and the fault is recorded with its source position instead of vanishing
silently.

### A whole class of memory bugs is gone

loft stores structs, vectors, and keyed collections on a managed heap. That heap had
a long tail of hard-to-see faults — a store leaked once per loop iteration, or a
value was freed while something still pointed at it (use-after-free) — clustered
around returning a value, reassigning one, and binding records out of `match` arms.
This release **retires that class**: whole-value binds copy and projections stay
views under one consistent rule, and the lifetime checker that decides when a store
is freed now reads a single source of truth instead of re-deriving it per site.

It stays fixed because three independent guards run continuously: a
poison-allocator test suite (every freed store is scribbled over, so any later read
is caught), an `Arbitrary`-driven program fuzzer, and a **nightly differential
oracle** that runs a growing corpus through *both* the interpreter and the `--native`
compiler and fails CI on any divergence in output, exit, or leak.

### Dense vectors and predictable copies

Vectors now default to dense storage, and the copy-versus-view model is spelled out
and enforced: a whole-value bind (`b = a`) **copies**, while a projection (`a.field`,
`v[i]` of a struct) stays a **view** onto the original. Narrowing casts are checked
rather than silently truncating.

### `&` — references you can write back through

A `&`-annotated binding creates a reference: pass `f(&x)` and the function can write
back to your `x`, or bind `a = &b` to alias a value for in-place update. It works on
scalars, heap values, and parameters, on both backends.

**Now including whole vectors.** A plain `d = v` (or `d = self.data`) *copies* the
vector — bind it, and the copy is independent. Write `d = &v` (or `d = &self.data`) and
you get a live, writable window instead: `d[i] = x` and `d += […]` write **through** to
the source. This is the ergonomic primitive for vector-heavy code — grab a sub-vector,
mutate it in place, no copy:

```loft
fn bump(self: Grid) {
  row = &self.cells;                 // a writable view of the field, not a copy
  for i in 0..len(row) { row[i] = row[i] + 1; }
}
```

The rule is one sentence now: **a plain bind copies; a `&` bind aliases** — the same for
scalars and vectors. (Reading a struct-typed field or element is still a view without
`&`, since it names an interior place, not a whole value.)

### Running untrusted code — the sandbox subset

A new compile-time **sandbox** lets you run untrusted loft with capability limits —
what a restricted caller may call, which parameters and fields it may touch, and what
it may mutate — enforced as a compile-time admission check, with an adversarial
escape suite proving the boundary holds.

### Games and the browser

`--html` gains **engine-less web modules** (a plain loft program compiles to a
self-contained WASM page), a `host_input()` primitive for feeding browser input in,
and asyncify resume that keeps running in headless / hidden tabs. A WebSocket WASM
bridge brings networked (and zero-trust crypto) programs to the browser.

### A pile of fixes

Among them: a keyed range or partial-key slice used as a value (`x = idx[lo..hi]`)
is now a clear compile error instead of a crash; `.map` on a literal receiver, a
nested-vector element-stride mismatch, and a native miscompile that returned an empty
vector from a struct field are all fixed. The four utility libraries touched by the
parse change — `arguments`, `random`, `regex`, and `cbor` — are migrated to the new
nullable-parse contract and republished.

## 2026-06

**New versioning.** Starting here, loft moves to a **monthly,
calendar-versioned cadence**: releases are named for their month — this one
is **`2026-06`** — which `Cargo.toml` spells `2026.6.0` (year.month.patch;
the patch digit is reserved for in-month security fixes). A deliberate step
up from the old `0.8.x` line.

This release rounds out loft's **library system** — toolchain-free native
libraries, signature-verified installs, and (with the namespace change below)
per-library namespaces instead of one shared flat space.

### Use a native library without a Rust toolchain

Native libraries (like `graphics` or `imaging`) used to compile their Rust
`cdylib` from source the first time you `use` them — needing `rustc`, `cargo`,
and the right system dev headers. loft can now fetch a **prebuilt** cdylib for
your platform and load it directly: no toolchain, no ~90-second first-use
compile. Building from source stays the automatic fallback when no prebuilt is
published for your platform.

### Registry installs are now signature-verified

`loft install` verifies the registry's index against a **trust root** embedded
in the loft binary before trusting any of it — every install is
cryptographically signed end to end, and a tampered index is refused. The trust
root is three independent keys, so a lost signing device can be retired without
disrupting anyone. Maintainers sign with a review-then-sign tool that shows
exactly what's going into a signature — re-downloading each library tarball to
confirm its checksum — before the key is ever used.

### Names that don't fight each other — enum-scoped variants, shadowing, import aliases

Naming got a lot less cramped (`@PLN22`):

- **Two enums can share a variant name.**  `enum Color { Red, Green }` and
  `enum Light { Red, Amber }` now coexist — a bare `Red` resolves from its
  context (the match subject, the declared type, a comparison, a function
  argument), and you can always qualify it as `Color.Red` when there's no
  context.  Defining a *new untyped variable* straight from a bare variant
  (`x = Red`) is a deliberate error — qualify it or give `x` a type — so adding a
  second enum with that variant can never silently re-point existing code.
- **Your names can shadow the standard library.**  `enum E`, `struct File`,
  `pub PI = 3` are all legal even though the stdlib already has `E` / `File` /
  `PI`; your definition wins bare lookup, and `std::E` still reaches the original.
  (The built-in *type* keywords — `integer`, `vector`, `iterator`, … — stay
  reserved.)
- **Import aliases.**  Rename a whole library or individual names on import:
  `use lib as m;` (qualifier `m::fn`), `use lib::Name as Alias;`, and grouped
  `use lib::(a as x, b, c);`.  Multiple names from one library must be
  parenthesised — `use lib::a, b;` is no longer accepted.

### Small integer types hold their full range — and won't silently null

The fixed-width integer types `u8`, `i8`, `u16`, `i16` pin a field to one or two
bytes; this release makes their ranges predictable and their edges safe.

- **`not null` gives the full native range.**  A `not null u16` now holds the
  whole `0..=65535` (before, `65535` read back as `null`); a `not null i8` holds
  `-128..=127` — exactly what the name promises.
- **A nullable field keeps one value aside for `null`**: `u8` is `0..=254` and
  `u16` `0..=65534` (the top trimmed), `i8` is `-127..=127` and `i16`
  `-32767..=32767` (kept symmetric).  Storing that one reserved value used to turn
  into `null` silently — now it's caught.  A literal is a compile error that tells
  you the fix (*"255 is reserved as the null sentinel of a nullable u8 (usable
  0..=254); declare the field `not null` for the full range, or cast with `as
  u8`"*); a value computed at run time gets a rate-limited warning that points you
  at the field while developing and stays quiet in a shipped game.
- **Narrow-element vectors match the fields.**  `vector<u16>` now holds `65535`
  and `vector<i16>` holds `32767`, just as `vector<u8>` already held `255`.

> Upgrading: if existing code stored the reserved edge value into a *nullable*
> narrow field (e.g. `255` into a `u8`), it will now flag instead of silently
> becoming `null` — declare the field `not null` for the full range, or cast.

### Windowed games without a server — `engine_host::run_local`

The games kernel gains a third way to run, next to the server (`run`) and the
network client (`run_client`): `run_local(tick_interval_us, on_event, on_tick)`
drives a **local windowed game** — steady ticks (one tick = one frame), the
kernel resting the CPU when nothing happens, and live build swaps — with no
server and no socket.  Close the window, call `client_stop()`, and the loop
returns.  When your game goes online later, you swap that one line for
`run_client` and keep your handlers exactly as they are.

### Window input as game events — `engine_host::post`

Post a local event from anywhere in your game — `engine_host::post("K:left")`
— and it arrives in your `on_event` handler like any network message
(`ev.cid == -1` tells you it came from this machine).  Key presses stop
slipping between frames, and your handlers no longer care whether input is
local or remote.  Servers with a window got their exit too: call
`engine_host::stop()` and `run` returns when the window closes.

### The debugger now tells you when a breakpoint can't work

Setting a breakpoint over `loft debug --rpc` answers with `verified` per
breakpoint: `false` means that line can never fire (no code on it, or a file
your program doesn't use) — so you find out immediately instead of waiting on
a stop that never comes.  Tracepoints also got friendlier: `"log": "expr"`
now works as a single expression (before, only the `["expr"]` array form did).

### An interactive prompt — `loft repl`

Run `loft` with no file (or `loft repl`) to get an interactive prompt where you
type loft one line at a time and see the result immediately:

```
loft> x = 40 + 2
loft> x
42
loft> fn dbl(n: integer) -> integer { n + n }
loft> dbl(x)
84
```

Names you bind stay available, functions and structs you define persist for the
session, multi-line input is supported, and a typo or run-time error doesn't end
the session.  Built-in commands inspect what you've defined — `:fns`, `:vars`
(each variable with its current value), `:bytecode`, `:rust`, `:slots` — and
`:help` lists them.

The prompt has **arrow-key history, in-line editing, and Tab completion** (of
function names, types, your variables, and `:`-commands), and it **remembers
your session**: the next time you start it, the variables and definitions from
last time are already there.  Start clean with `loft repl --fresh`.  See
[doc/claude/REPL.md](doc/claude/REPL.md).

### Look inside a program — `loft introspect`

`loft introspect <file>` prints a program's bytecode, the Rust loft generates
for it, per-function variable slot tables, and inferred types — side by side, in
one command.  Sub-flags pick one view (`--show-bytecode`, `--show-rust`, …) or a
single function (`--fn`).  This replaces hunting through `LOFT_LOG=…` dumps for
everyday inspection.

---

## 0.8.5 — 2026-06-07 — Language Maturity

This release is about the language itself getting solid.  Closures finally
work the way you'd expect, bounded generics carry types correctly through
methods and tuples, the native backend ships as production, and a
browser-based **branch review viewer** lets you read your in-flight code
from any device with `make view` + an SSH port-forward.

### Closures that capture what they should

The biggest user-visible fix.  Closures now hold a **live reference** to
captured variables, not a snapshot — they see the latest value, mutations
through one closure are visible to another, and a closure that captures a
struct field reads the field as it currently is.

```loft
counter = make_counter()  // returns a closure pair (inc, get)
counter.inc(); counter.inc(); counter.inc()
println(counter.get())    // 3 — was 1 before this release (snapshot bug)
```

- Closures returned from functions keep their captured environment alive.
- Multiple closures sharing the same captured cell see each other's
  writes.
- Closure-captured vector / struct / nested-struct fields read live, not
  stale.
- Validation matrix in `tests/closure_matrix.rs` cross-checks 30+ shapes
  on interpreter + `--native`.

### Bounded generics + interfaces

Write generic functions with type constraints; the compiler picks the
right per-type implementation at the call site.

```loft
fn show_pair<T: Printable>(a: T, b: T) -> text {
    "{a.to_text()} & {b.to_text()}"
}
println(show_pair(3, 7))           // 3 & 7   (built-in to_text)
println(show_pair("hi", "ho"))     // hi & ho (text passes through)
```

- `<T: Bound>` constraints — `Ordered`, `Equatable`, `Addable`, `Numeric`,
  `Scalable`, `Printable`, plus user-defined interfaces.
- Bound-typed values round-trip through tuples, vectors, struct fields,
  and `for` loops — the compiler now substitutes T's concrete type
  everywhere it appears.
- Generic functions returning `(T, T)` work with text, references, and
  user types — not just primitives.
- Format-string interpolation `"{x}"` where `x: T` routes through the
  bound's `to_text` method automatically.

### Tuples cross-validated end-to-end

Tuples now ship as a fully validated value type.  40 cross-mode test cells
cover 5 element types (scalars, text, nested tuples, closures, struct
references) across 3 storage destinations (local, direct stack, struct
field) — interpreter and `--native` produce byte-identical output.

```loft
fn split_message(s: text) -> (text, text) {
    n = s.len() / 2
    (s[0..n], s[n..s.len()])
}
left, right = split_message("hello world")
```

### Branch review viewer (`make view`)

`make view` launches a browser-accessible doc + code review surface for
the current loft branch.  Dashboard shows files changed vs `main`,
recent commits, uncommitted state — all with status badges.  Click any
file for a rendered view (`.md` rendered via the new `lib/markdown`
library, others as line-numbered code), toggle between
`Rendered ¦ Diff vs main`, click any commit for the per-file diff,
click any tracker tag (`@P-id` / `@PLAN-id`) for cross-doc references.
SSH-port-forward 8765 from the host.  Built entirely in loft (web
server, markdown rendering, JSON parsing, file walking) + a small bash
wrapper for `git` calls; no Python, no external markdown library, no
syntax-highlighter dependency.  See
[doc/claude/DEBUG.md § Branch review viewer](doc/claude/DEBUG.md#branch-review-viewer-make-view).

A `/welcome` landing page surfaces project status at a glance: open
problems, recently closed bugs (last 30 days), active and recently
finished plans, future plans by category — all built from a live tracker
index that updates on every commit.

### Tracker index (`make index`)

A small file-based index of every `@P<id>` / `@PLAN<id>(-segment)*`
reference across the project, queryable from the command line.  The
viewer surfaces the same data; CI uses it to catch broken tracker
references at commit time.

```bash
make index                            # rebuild index/tags.json
./scripts/idx tag:@P259               # all references to a P-issue
./scripts/idx prefix:@PLAN37          # all PLAN37-* phase refs
./scripts/idx incoming:doc/claude/PROBLEMS.md   # backlinks to a doc
./scripts/idx broken                  # broken @-references
./scripts/idx broken-links            # broken markdown links
```

A loft-native scanner port (`make index-loft`) reproduces the bash
scanner's output via the loft language itself — exercises long-running
file-walking + JSON emission shapes that no other loft program touches.

### Native compilation goes production

The `--native` backend (loft → Rust → rustc → standalone binary) is now
the default.  108 / 108 native tests pass; closures, generics, tuples,
JSON, and the viewer all compile + run identically under `--native` and
`--interpret`.  Use `--interpret` only when bisecting a native-only
regression.

Eight previously-tracked native codegen bugs closed (use-after-free in
heap-typed tail returns, text-concat type-dispatch, generic vector
struct returns, closure-tuple-field layout, parallel-queue native
runtime, and four more).

### `lib/markdown` — markdown renderer in loft

A standalone library: headings, bold, italic, inline code, fenced code,
links with anchor support, tables (with alignment), lists (ordered +
unordered + nested), images, autolinks for tracker tags, autolink
prefix configuration, image-URL rewriting.  Pure loft — no external
parser.  Used by the branch-review viewer and any future loft
documentation tool.

```loft
use markdown
html = markdown::render(source, "/tag/", "/img/", "")
```

### Smaller language wins

- **`@P274`** — `text + integer` concat now correctly converts the
  integer (was emitting `OpAppendText` with a raw i64; SIGSEGV in
  interp / E0614 in native).
- **`@P275`** — module-scope `const vector<T>` works under the
  default `--native` path (was only initialised under
  `--native-release`; default emit panicked at
  `stores.const_refs[NNN]`).  Side-fix: nested `OpConstRef` calls
  no longer accumulate `stor` prefixes in their substituted form
  (a substring-of-its-own-output bug in the codegen template
  rewriter).
- **`@P276`** — `(s[i] ?? '<c>') == '<c>'` now type-checks under
  `--native` (was rustc E0308: the pre-evaluated block holding
  the character lifted as `i32`, then the outer
  `OpConvIntFromCharacter` template compared it against `char`).
  Bind-then-compare (`c = s[i] ?? '*'; if c == 'b'`), else-if
  chains, and ordering compares (`<`/`>`) all work too.
- **`@P283`** — format-string interpolation of a self-slice-
  reassigned text PARAMETER no longer crashes either backend.
  Pattern: `fn f(rb: text, id: text) -> text { …; rb = rb[a..b];
  "[{id}] {rb}" }` was SIGSEGVing the interpreter and rejecting
  with rustc E0368 in native.  The work-buffer parameter
  promoted by `text_return` is `RefVar(Text)` (`&mut String`),
  but the codegen for `OpAppendText` / `OpClearText` /
  `OpFormatText` / `OpFormat{Int,Float,Single,Database}` /
  `OpAppendCharacter` on these targets emitted the local-String
  variants — interp treated the refvar slot as a `String` →
  SIGSEGV; native emitted `var += &*(…)` on `&mut String` →
  E0368.  Fix dispatches to the matching `Stack` variant for
  RefVar(Text) targets on both backends (mirrors the existing
  B7 OpAppendCharacter dispatch).
- **`@P259`-`@P261`** — closure / store-allocation / vector-field
  fixes (the closure-cell trio).
- **UTF-8** — `json_parse` now decodes 2/3/4-byte UTF-8 codepoints
  correctly (was widening byte-by-byte; `→` became `âââ`).
- **WebSocket binary frames** — `lib/server` exercises the binary
  path in production; multi-client games use it.
- Eight new P-issues filed from dogfood discovery (native codegen +
  parser quirks surfaced by writing real loft consumers); fixes
  scheduled across the next few releases.

### Workflow + project-management

- New `## Open work` sections in reference docs catalog
  enhancement opportunities discovered while building real consumers.
- DEVELOPMENT.md documents the "fix-on-the-spot vs canonical-home"
  workflow for handling discovered language gaps mid-feature work.
- Plan documentation reorganized: `plans/` for core-language work
  (capped at 2-3 active), `lib_plans/` for library work, `ROADMAP.md`
  as the prioritization view.
- Every PROBLEMS.md row now self-tags with `**@P<n>**` so the
  index unambiguously links each row to its references.

### Relative file paths are now program-relative — portable "program + assets" bundles

A relative file path — `file("assets/font.ttf")`, `read_file("data.bin")`,
`delete("out.tmp")` — now resolves against **the program's own directory** (the
source dir under `--interpret`, the executable's dir under `--native`), not the
process working directory.  An asset addressed relative to your program loads no
matter where the program is launched from:

```loft
f = file("assets/level1.dat");   // beside the program, wherever it runs from
```

This is what #255 needed: a bundled font worked from the source tree but vanished
under `--native` (which runs from a temp dir), because the path resolved against
the cwd.  **Absolute paths are never rewritten.**  Resolution is uniform across
`file()`, `exists()`, `read_file`/`write_file`, the `File` methods,
`delete`/`move`/`mkdir`, and image loads.

**CLI tools opt back into cwd** with a one-line file-top directive — a
*user-supplied* relative path then resolves against the working directory:

```loft
#cwd
fn main(args: vector<text>) { data = read_file(args[1]); }
```

Per-invocation, `LOFT_PATHS=program` / `LOFT_PATHS=cwd` overrides both.
`source_dir()` returns the anchor and now works under `--native` (was empty
before).

**Breaking change** — a program that read or wrote a relative path expecting the
*working directory* now needs `#cwd` at the top.  The in-tree corpus that did so
(13 file-I/O tests) was migrated in this release.

### Faster startup, automatically — the program cache is on by default

Running the same program again is now **~3× faster to start**: the first
run caches the fully-parsed program (the standard library, every library
you `use`, and your script) next to your other caches, and later runs of
the unchanged program skip parsing entirely.  It just works — no flag to
set.  If anything the program reads changes, the cache notices and
re-parses, so you never get a stale result.

- **Turn it off** with `LOFT_NO_CACHE=1` (e.g. for one-shot batch jobs
  where the first-run save isn't worth it).
- **Cap its size** with `LOFT_CACHE_MAX_MB` (default 512 MiB); the oldest
  bundles are evicted past the limit.
- It automatically stays **off inside `cargo run` / `cargo test`**, so
  building the compiler never serves a stale parse.

### File `+=` is now append-only — and `file.sync()` lets you flush

`f += value` now **appends** to the end of the file, matching how
`vector += [elem]` and `text += "more"` work on the other collection
types.  Earlier writes are preserved when you re-open the file:

```loft
{f = file("log.txt"); f += "first\n";  f.sync(); }
{f = file("log.txt"); f += "second\n"; f.sync(); }
{f = file("log.txt"); f += "third\n";  f.sync(); }
// Result: 19 bytes — "first\nsecond\nthird\n", not just "third\n".
```

Use `f.sync()` between log records or block boundaries to guarantee
the buffered bytes have landed on disk before the next write is
issued.  Returns `true` on success; on `Directory` / `NotExists` the
call short-circuits to `false`.

**Breaking change** — code that relied on `f += …` truncating the file
on first re-open now needs to call `f.set_file_size(0)` (or
`f#size = 0`) explicitly before the first write.  Updated call sites
in this release: `tools/audience-demo/single_port_server.loft`,
`lib/world/src/world.loft`, `lib/graphics/src/glb.loft`,
`scripts/build-playground-examples.loft`.  Explicit offsets via
`f#next = N` still overwrite at offset `N`, so the snapshot idiom
(fixed-slot headers, overwrite-in-place) keeps working.

### Interpreter no longer corrupts memory on deep recursion

The interpreter's value stack now grows on demand.  Previously it was
a fixed 8 KB buffer that never expanded, so a program that nested
function calls deeply enough (roughly 40+ frames carrying a handful of
locals) would silently write past the buffer and corrupt the heap —
usually surfacing as a confusing "double free or corruption" abort
*after* the program had finished printing its output.  Deeply
recursive interpreted programs now run correctly (the `--native`
backend was never affected, as it uses the real machine stack).

## 0.8.4 — 2026-04-24 — Awesome Brick Buster

This release focuses on **the web**: your loft programs can now fetch
URLs, serve HTTP, parse JSON, and even run entirely inside a browser tab.
The headline is **Brick Buster** — a full arcade game, paddle + ball +
powerups + music + levels + high score, that you can share with a friend
via a single URL.

### JSON — read and write structured data

```loft
v = json_parse("{\"name\":\"Alice\",\"age\":30}")
println(v.field("name").as_text())   // Alice
println(v.to_json_pretty())          // formatted output
```

- `json_parse(text)` turns JSON into a value you can explore.
- Bad input returns a null value instead of crashing.  Ask
  `json_errors()` what went wrong.
- Build JSON from code with `json_number`, `json_string`,
  `json_array`, `json_object`, ...
- Read it back with `field("key")`, `item(index)`, `len()`, `keys()`.
- `MyStruct.parse(json_value)` fills a struct from JSON in one line.

### HTTP — talk to the web

```loft
use web
r = http_get("https://example.com")
if r.ok() { println(r.body) }
```

- `http_get`, `http_post`, `http_put`, `http_delete` — straightforward
  blocking calls that return an `HttpResponse` with `.status`, `.body`,
  and `.ok()`.
- `..._h` variants accept custom headers: `http_get_h(url, ["Accept: application/json"])`.
- A simple HTTP **server** is also available: `for req in listen(8080) { respond(req, ...) }`.

### Lighting that actually lights

The 3D renderer's PBR shader now uses the light colours and intensity
you pass in.  Previously the `Light` struct was accepted by the
scene-graph but the shader ignored `color_r/g/b`, `intensity`, and all
point lights — every scene looked as if lit by a single neutral-white
directional.

- A directional light's `intensity` scales its contribution.
- A scene's first **point light** is now rendered (quadratic
  attenuation; no shadow yet).
- Goldens for five of the graphics examples are checked in as
  regression guards — a shader tweak that breaks lighting is caught by
  a pixel-diff test.

### Games in the browser

- **Brick Buster** — a complete arcade game (paddle, ball, powerups,
  music, levels, high score) that runs in your browser and on the
  desktop.  Try it at
  <https://loft-lang.org/loft/brick-buster.html>.
- **Graphics gallery** — 24 WebGL demos, from hello-triangle to
  physically-based rendering.
- `loft --html program.loft` produces a single folder you can drop on
  any static web host.

### Easier code, clearer errors

- `parallel { }` really runs in parallel now (one OS thread per arm).
- `x ?? return err` — one line instead of a two-line null check.
- `type Handler = fn(Request) -> Response` — name function and tuple
  types.
- Any type with `fn next(self) -> Item?` can be used in `for x in val`.
- When the interpreter hits a fatal error, it now tells you *which
  function and line* triggered it before exiting.

### A gentler language

- `integer` is now 64-bit everywhere.  Big numbers like
  `9_876_543_210` just work — no suffix required.
- The old `long` type and `33l` literal suffix are gone; use `integer`
  and `33`.
- Three crashes involving `match` on complex types are fixed —
  character interpolation, uneven match arms, and chained native calls
  no longer leak memory.

### Native editor & tooling

- **Native Moros editor** — a full OpenGL editor ships as a standalone
  app you can distribute without installing loft.
- `loft --dump file.loft` — show the compiled bytecode without running
  the program.  Handy when something compiles oddly.
- New test runner: `scripts/find_problems.sh --bg` runs the whole suite
  in the background; check in with `--peek` or `--wait`.

---

### Closures you can return

Functions that return a closure now work correctly, including when the
closure captures variables from the enclosing scope:

```loft
fn make_greeter(prefix: text) -> fn(text) -> text {
    |name| { "{prefix}, {name}!" }
}
hello = make_greeter("Hi")
println(hello("Ada"))   // Hi, Ada!
```

Capturing closures also work with `map` and `filter`:

```loft
factor = 10
big = map(nums, |x| { x * factor })
```

### Quality-of-life fixes

- **Typos stop compilation.**  `y = unknown_thing` now fails with a
  clear error instead of silently creating a garbage value.
- **`rev(vector)`** — you can now iterate a plain vector in reverse.
- **Format strings** — `"{n:<5}"` (left-align), `"{n:^5}"` (centre) and
  `"{f:.0}"` (zero decimals) all behave the way you'd expect.
- **File reading** — `file.lines()` now returns text after the last
  newline, not just full lines.
- **Sorted collections** — descending primary-key ranges return
  correct results in every mode.
- **Windows paths** — native compilation correctly escapes `\` in file
  paths.

### Faster programs

- The compiler does arithmetic at compile time where it can, so
  `[for i in 0..100 { i * 2 }]` becomes a ready-made vector instead of
  a loop.
- `par(...)` automatically picks a lighter, faster worker when your
  work doesn't need its own scratch memory — no syntax change.

### Better docs

- New pages on **pattern matching** and **format strings**.
- Expanded chapters on images, threading, and generics.
- 137-page PDF reference regenerated.

---

## 0.8.3 — 2026-03-27 — WebAssembly!

Loft now runs in the browser.  The playground at
<https://loft-lang.org/loft/playground.html> compiles and executes
loft programs entirely in your browser tab — no server involved.

Behind the scenes:

- A virtual in-memory filesystem for browser tests.
- Captured `println` output for the playground.
- A stable plugin protocol so native extensions (imaging, random, web)
  can be loaded at runtime.
- String-heavy programs are faster thanks to format-string
  pre-allocation.

---

## 0.8.2 — 2026-03-24

### Lambdas

Write throw-away functions inline:

```loft
doubled = map([1, 2, 3], |x| { x * 2 })
```

The short form `|x| { ... }` infers types from where you use it.  Use
the long form `fn(x: integer) -> integer { ... }` when you want them
explicit.

### Named arguments and defaults

```loft
fn connect(host: text, port: integer = 80, tls: boolean = true) { ... }

connect("localhost")                       // uses both defaults
connect("localhost", tls: false)           // skips port by name
```

### Native compilation

Ship your loft program as a real native binary:

- `loft --native file.loft` — compile and run via `rustc`.
- `loft --native-emit out.rs` — save the generated Rust source.
- `loft --native-wasm out.wasm` — compile to WebAssembly.

### JSON, computed fields, field constraints

- `"{value:j}"` serialises any struct to JSON.
- `Type.parse(json_text)` parses JSON back into a struct.
- `computed(expr)` fields are recalculated on every read, no storage
  needed: `area: float computed(PI * $.r * $.r)`.
- `assert(...)` clauses on struct fields validate every write.

### Small but welcome

- Workers started with `par(...)` can now return `text` and enum
  values, not just numbers.
- `fn` prefix dropped on function references: write `apply(double, 7)`,
  not `apply(fn double, 7)`.
- `pub` is now required to expose a definition to other files — this
  keeps your module boundaries tidy.

### Clearer errors

- Using `string` as a type suggests `text` instead of a generic error.
- Six common mistakes now come with a fix suggestion.
- Several crashes on unusual input have become proper error messages.

### Bug fixes

- `c + d` on two characters now produces text, not a crash.
- Empty vector literal `[]` as an argument no longer crashes.
- `v += other_vec` on text-bearing vectors no longer corrupts data.
- `map`, `filter`, and `reduce` no longer trip over their own internal
  slots.

---

## 0.8.0 — 2026-03-17

### Match expressions

Pattern-match enums, structs, and scalars:

```loft
match shape {
    Circle { r } => PI * pow(r, 2.0),
    Rect { w, h } => w * h,
}
```

- The compiler checks that you cover every case.
- Supports `North | South =>` (or-patterns), `if r > 0.0` (guards),
  `1..=9` (ranges), null patterns, character patterns, and full
  `{ ... }` block bodies.

### Formatter

- `loft --format file.loft` — format in place.
- `loft --format-check file.loft` — fails if not formatted; useful in
  CI.

### Imports

- `use mylib::*` — bring in everything.
- `use mylib::Point, add` — pick out just what you need.
- Local definitions always win over imported ones.

### Higher-order helpers

```loft
doubles = map(numbers, fn double)
evens   = filter(numbers, fn is_even)
total   = reduce(numbers, fn add, 0)
```

### Testing made easier

- `loft --tests file.loft::test_name` — run a single test.
- `loft --tests 'file.loft::{a,b}'` — run a selection.
- `loft --tests --native` — compile tests to a native binary first.

### New standard-library helpers

- `now()` — milliseconds since 1970.
- `ticks()` — microseconds since program start, monotonic.
- `mkdir(path)` / `mkdir_all(path)` — make directories.
- `vector.clear()` — empty a vector.

### Clearer warnings

- Division or modulo by a constant zero.
- Unused loop variables (silence with `for _i in ...`).
- Unreachable code after `return`, `break`, or `continue`.
- Redundant null checks on `not null` fields.

### Bug fixes

- `x << 0` and `x >> 0` now return `x` instead of null.
- `NaN != x` now returns `true` (it was wrongly `false`).
- `??` works correctly with floats.
- Using `if` as an expression without `else` is now a compile error
  rather than silently returning null.
- Assigning `null` to a struct field no longer crashes.
- `sorted[key] = null` and `hash[key] = null` remove the entry, as
  documented.

---

## 0.1.0 — 2026-03-15 — First release

The core language, in one place.

### Types and values

- **Static types with inference** — no type annotations on locals; the
  compiler figures out the type from the first assignment.
- **Null safety** — every value may be null unless declared `not
  null`; null propagates through arithmetic; `?? default` supplies a
  fallback.
- **Primitives** — `boolean`, `integer`, `long`, `float`, `single`,
  `character`, `text`.
- **Structs** — named records: `Point { x: 1.0, y: 2.0 }`.
- **Enums** — both plain enums and struct-enums (variants with fields
  and per-variant methods).

### Control flow

- `if`/`else`, `for`/`in`, `break`, `continue`, `return`.
- For-loop extras — inline filter (`for x in v if x > 0`), loop
  attributes (`x#first`, `x#count`, `x#index`), in-loop removal
  (`v#remove`).

### Working with collections

- `[for x in v { expr }]` — vector comprehensions.
- `vector<T>` (dynamic array), `sorted<T>` (ordered tree),
  `index<T>` (multi-key tree), `hash<T>` (hash table).

### Text and formatting

- `"Hello {name}, score: {score:.2}"` — string interpolation with
  format specifiers.

### Other

- **Parallel execution** — `for a in items par(b=worker(a), 4) { ... }`
  spreads the work across CPU cores.
- **File I/O** — read, write, seek, directory listing, PNG images.
- **Logging** — `log_info`, `log_warn`, `log_error` with source
  location and rate limiting.
- **Libraries** — `use mylib;` imports from `.loft` files.

---

## Version comparison links

- [Unreleased vs 2026-06](https://github.com/loft-lang/loft/compare/v2026.6.0...main)
- [2026-06 (2026.6.0)](https://github.com/loft-lang/loft/compare/v0.8.5...v2026.6.0)
- [0.8.5](https://github.com/loft-lang/loft/compare/v0.8.4...v0.8.5)
- [0.8.4](https://github.com/loft-lang/loft/compare/v0.8.3...v0.8.4)
- [0.8.3](https://github.com/loft-lang/loft/compare/v0.8.2...v0.8.3)
- [0.8.2](https://github.com/loft-lang/loft/compare/v0.8.0...v0.8.2)
- [0.8.0](https://github.com/loft-lang/loft/releases/tag/v0.8.0) — the first tagged release
- 0.1.0 — pre-dates tagging (no release tag exists)
