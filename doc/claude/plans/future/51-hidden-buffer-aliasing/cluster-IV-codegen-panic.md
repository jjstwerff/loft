# Cluster IV — Codegen panic on two-heap-returning-branches

**Severity:** Worst class — hard panic halts compilation on BOTH backends.
**Affected probes:** 08, 13, 22, 27, 33 (5 probes)
**Failure site:** `src/state/codegen.rs:2529:9`

## The assertion that panics

```rust
pub(super) fn generate_var(&mut self, stack: &mut Stack, variable: u16) -> Type {
    assert!(
        stack.function.stack(variable) <= stack.position,
        "Incorrect var {}[{}] versus {} on {}",
        stack.function.name(variable),
        stack.function.stack(variable),
        stack.position,
        stack.data.def(stack.def_nr).name
    );
    ...
}
```

The check: a variable's STACK OFFSET (`stack.function.stack(variable)`) must be `<= current eval stack position`.  In the panicking cases the offset is `65535` (= `u16::MAX`), the **null sentinel** for "this variable has NO ASSIGNED STACK SLOT".

The panic message format:
```
Incorrect var __ref_2[65535] versus 136 on n_main
                ^^^^^^ ^^^^^^      ^^^      ^^^^^^
                name   offset      pos      function
```

## What this means

`generate_var` is the codegen routine that emits a `VarRef`/`VarInt`/etc. opcode to read a variable's value at the current PC.  Before emitting, it asserts the variable HAS a valid stack offset.

When the offset is `u16::MAX`, the variable was either:

1. **Never allocated a stack slot** by `set_function_stack_size` (or wherever stack layout is determined).
2. **Removed by an earlier pass** (e.g. `unify_if_branches_work_refs` dropping a redundant work-ref) but a reference to it survived in the IR.

Hypothesis (2) is consistent with the trigger shape — all 5 panicking probes have two heap-returning code paths converging via `if` / recursion.  @P236's `unify_if_branches_work_refs` is designed to pick ONE shared work-ref for both branches and substitute the other into it.  If the substitution misses a reference site (or runs but doesn't propagate to ref_return / main's call-site arg generation), the dropped work-ref is still cited.

## Reference probe — 18 (match-tail, PASSES)

```loft
fn render_match(p: P) -> Canvas {
  match p.tag % 3 {
    0 => alloc_canvas(4, 5, 100),
    1 => alloc_canvas(4, 5, 200),
    _ => alloc_canvas(4, 5, 300),
  }
}
```

**Lowered IR** (`/tmp/bc_18.txt` line 1316):

```
fn n_render_match(p:P, __ref_1:Canvas, __ref_2:Canvas, __ref_3:Canvas)
    -> Canvas["__ref_1", "__ref_2", "__ref_3"]
{
  [26] return {#scalar_match(2):ref(Canvas)
    _match_subj_1(2):integer = OpRemInt(OpGetInt(p(0), 0i32), 3i32);
    if OpEqInt(_match_subj_1(2), 0i32)
      n_alloc_canvas(4, 5, 100, __ref_1(0))
    else if OpEqInt(_match_subj_1(2), 1i32)
      n_alloc_canvas(4, 5, 200, __ref_2(0))
    else
      n_alloc_canvas(4, 5, 300, __ref_3(0));
  };
}
```

**Key observation:** match-tail has **THREE hidden buffer parameters** (`__ref_1, __ref_2, __ref_3`), one per arm.  Each arm's call gets its own buffer.  No unification.  The function's return-type deps `["__ref_1", "__ref_2", "__ref_3"]` lists all three.

Main's caller (probe 18 main):

```
fn n_main() {
  __ref_3(1):ref(Canvas) = null;
  __ref_2(1):ref(Canvas) = null;
  __ref_1(1):ref(Canvas) = null;
  ...
  [36] cv(5):ref(Canvas) = n_render_match(p(5), __ref_1(1), __ref_2(1), __ref_3(1));
  ...
  OpFreeRef(__ref_1(1));
  OpFreeRef(__ref_2(1));
  OpFreeRef(__ref_3(1));
}
```

Main allocates THREE work-refs (one per buffer attr), passes them all, frees them all.  ✅ Clean.

## Problem probe — 08 (if-tail, PANICS)

```loft
fn render_if(p: P) -> Canvas {
  if p.tag % 2 == 0 {
    alloc_canvas(4, 5, p.tag)
  } else {
    alloc_canvas(4, 5, p.tag * 10)
  }
}
```

**Cannot dump the IR** — codegen panics before any dump output.  The panic message:

```
thread 'main' panicked at src/state/codegen.rs:2529:9:
Incorrect var __ref_2[65535] versus 136 on n_main
```

So in `n_main`, when emitting opcodes to call `render_if(p, __ref_2)`, the codegen tries `generate_var(__ref_2)` but `stack.function.stack(__ref_2) == 65535` (unallocated).

## Hypothesised mechanism

`unify_if_branches_work_refs` (`src/parser/control.rs:721`) runs in `block_result` for tail-position `If` returning a heap type.  It picks the FIRST branch's terminal work-ref as the "shared" one and rewrites the OTHER branch to use it.  Result: ONE shared work-ref instead of two.

After unification, `render_if` should have ONE hidden buffer attribute (instead of two).  ref_return promotes that one to the signature.

**Where it breaks:** when main parses `cv = render_if(p);`, it synthesizes `add_defaults` to fill in hidden buffer args.  If `add_defaults` synthesizes work-refs based on the COUNT of hidden buffer attrs in render_if's signature, and the count is one after unification — but somehow main ends up with TWO work-refs (`__ref_1` and `__ref_2`) anyway, with `__ref_2` never properly registered with `set_function_stack_size`.

**Likely path:** the parser detects render_if has two heap-returning branches and pre-allocates two work-refs in main BEFORE `unify_if_branches_work_refs` runs.  Unification removes one buffer from render_if's signature but doesn't remove the corresponding work-ref from main.  Result: `__ref_2` exists in main's variable table but doesn't get a stack slot because nothing references it post-substitution.

## Why match (probe 18) doesn't trigger this

Match takes a different IR path:

- match arms are wrapped in `Value::Block` containing a `scalar_match` construct (`#scalar_match` in the dump).
- ref_return promotes each arm's work-ref independently (no unification).
- Main synthesizes one work-ref per arm, all get stack slots, all get freed.

If-tail / explicit-return-in-if / recursion all engage `unify_if_branches_work_refs` (or attempt to), which is the broken path.

## What we know vs. don't

| | Status |
|---|---|
| The assertion location | ✅ Read at `src/state/codegen.rs:2529-2536` |
| The trigger shape | ✅ Five probes confirm: BOTH branches must be heap-returning |
| Match escapes the bug | ✅ Probe 18 vs 08 — same logical shape, different IR path |
| One-branch escapes the bug | ✅ Probe 23 — only one branch is heap; one-branch panic doesn't fire |
| `unify_if_branches_work_refs` is implicated | 🤔 Strong hypothesis, not verified by source reading |
| Exact substitution miss | ❌ Not pinned — needs reading of `unify_if_branches_work_refs` body and main's `add_defaults` interaction |
| Whether recursion (probe 13) has the same mechanism | 🤔 Likely (same panic site, same shape family), but recursion's IR is distinct |

## Investigation tasks

1. **Read `unify_if_branches_work_refs`** at `src/parser/control.rs:721-768`.  Walk through what it does step-by-step on probe 08's body.
2. **Read `add_defaults`** at `src/parser/mod.rs:3674` (cited by Explore agent earlier).  Trace how main synthesizes hidden buffer args for the call to render_if.
3. **Read `set_function_stack_size`** (wherever it lives) — when does a variable's stack offset get set vs. left at u16::MAX?
4. **Add a temporary eprintln** in `unify_if_branches_work_refs` to print "[unify] in FN: shared=X, dropped=Y" — see if it fires on probe 08.
5. **Compare bytecode emission for probe 08 vs 18** side-by-side once we can dump probe 08's IR (would need a workaround to bypass the panic — maybe a `--dump-ir-only` flag that runs the parser but skips codegen).

## Fix surface

Two possible approaches:

**(a) Make if-tail behave like match-tail.**  Stop unifying; promote each branch's work-ref independently.  Each becomes a separate hidden buffer attribute.  Main passes one work-ref per attribute, all freed cleanly.  Effort: depends on what unify_if_branches_work_refs was solving in @P236 — re-evaluate whether unification is necessary or if multi-buffer match-style would suffice.

**(b) Fix unification's reference-site updates.**  Wherever `unify_if_branches_work_refs` substitutes the dropped work-ref, ensure ALL reference sites (including caller-side `add_defaults` and `set_function_stack_size` enumeration) see the substitution.  Effort: targeted, but requires understanding the substitution propagation.

**(c) Defer (Path C — refcount).**  Cluster IV is a parse-time / scope-analysis bug, not a runtime-ownership issue.  Path C wouldn't directly fix it; this stays as its own targeted patch even if Path C lands.

Most likely outcome: (b) — finish `unify_if_branches_work_refs` to handle the missed cases.  Could be small if it's a single missing substitution site.
