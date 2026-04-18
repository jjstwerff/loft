# Phase 2 — audit adjacent OpCopyRecord emission sites

Status: **Done — clean audit, no additional fixes needed.**

## Scope

Enumerate every `OpCopyRecord` emission site across the codebase, classify each
by (a) whether it ever sets the `0x8000` free-source flag, (b) whether it
wraps the copy in a `n_set_store_lock` bracket, (c) how it gates the flag when
applicable.  Cross-reference with the historical fix lineage
(P143/P150/P152/P155/P171) to confirm every known shortcut has been patched
or is inherently safe.

## Inventory

### Bytecode path (`src/state/codegen.rs`)

| # | Site | Line | Sets 0x8000 | Lock bracket | Gate |
|---|---|---|---|---|---|
| 1 | `generate_set` reassignment | ~924 | Conditional | Yes | `has_hidden_ref || is_borrowed_view` |
| 2 | `gen_set_first_ref_var_copy` | ~1206 | Never | No | — (var-to-var, no callee free to manage) |
| 3 | `gen_set_first_ref_tuple_copy` | ~1227 | Never | No | — (tuple elements already materialised by their own copy path) |
| 4 | `gen_set_first_ref_call_copy` | ~1284 | Conditional | Yes | `is_borrowed_view` (Phase 1 + 1b) |

Sites 1 and 4 are the P181-affected ones, both gated on
`is_borrowed_view = !def.returned.depend().is_empty()`.  Phase 1b's
`parse_return` merge ensures the gate sees a complete dep chain for
mixed-return callees.

Sites 2 and 3 never emit `0x8000`, so there is no path by which they could
free a caller's store.  They are safe by construction.

### Native path (`src/generation/dispatch.rs`)

| # | Site | Line | Sets 0x8000 | Lock bracket |
|---|---|---|---|---|
| 5 | Call-source first-assignment | ~74 | Never | No |
| 6 | Var-source reassignment | ~110 | Never | No |

Native-path emitters never set the flag.  Runtime interpretation happens at
`src/codegen_runtime.rs::OpCopyRecord` (P171 brought this to parity with
the bytecode handler).  The dispatch layer could in principle emit
`tp | 0x8000` to opt into freeing — today it never does, so the sites are
safe.  **Phase 2b** tracks whether this stays true as native codegen
evolves.

### Runtime handler (`src/state/io.rs::copy_record`)

Not an emission site — receives `raw_tp` from the stack, masks the high
bit, and frees the source conditionally on `!locked && store_nr != 0 &&
store_nr != to.store_nr && !free`.  The `locked` check is what the
`n_set_store_lock` bracket at emission sites 1 and 4 relies on.

## Historical fix cross-reference

| P-ID | Site | Fix |
|---|---|---|
| P143 | Site 4 | Introduced lock bracket on ref-typed call args |
| P150 | Site 2 | Confirmed no free needed on var-to-var copies |
| P152 | `parser/expressions.rs` | Vector-field reassignment RHS capture (unrelated to OpCopyRecord flag) |
| P155 | Site 1 | Back-ported P143's lock bracket to the reassignment path |
| P171 | `codegen_runtime.rs` | Added 0x8000 mask + claim cleanup + free-condition check to native runtime |
| P181 Phase 1 | Sites 1, 4 | Added `is_borrowed_view` gate |
| P181 Phase 1b | `parser/control.rs::parse_return` | Merge mid-body return deps so Phase 1's gate sees complete view info |

Every historical P-ID touching `OpCopyRecord` has a landed fix at the
emission or handler site.  No open items.

## Empirical confirmation

One additional snippet added to widen the probe surface:

- `snippets/18_tuple_destructure.loft` — destructure a tuple whose elements
  are results of struct-returning calls on field-access args.  Tests that
  site 3 (tuple_copy) remains safe even when the elements individually went
  through site 4 (call_copy).  **PASS**.

All 17 pre-existing variants continue to pass.

## WASM note

Site 4's non-WASM conditional `0x8000` emission is disabled entirely under
the `wasm` feature (unconditional `i32::from(tp_nr)`).  This means under
WASM, callee-fresh stores are never freed by the emission-site shortcut
and rely on other cleanup.  Not a corruption risk — at most a leak — but
worth noting for future WASM leak audits.  Out of scope here.

## Conclusion

Phase 2 closes clean.  No new bugs found; no fix required.  The
inline-lift safety invariant holds at every OpCopyRecord emission site in
the bytecode path.  The Vector arm that was deliberately skipped in Phase
1b remains a conditional Phase 1c item; no crash variant has been observed
to require it.
