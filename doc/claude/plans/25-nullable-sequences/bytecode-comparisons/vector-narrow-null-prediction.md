# vector<narrow?> null — fix prediction (2026-07-10)

Working reference = the `u8?` **FIELD** template (proven working):
- write: `OpSetByteNullable(rec, 0, 0, <val | OpConvIntFromNull()>)`
- read:  `OpGetByteNullable(rec, 0, 0)`
- test:  `OpEqInt(OpGetByteNullable(...), OpConvIntFromNull())`

## Prediction: after the fix, `vector<u8?>` introspect shows (matching the field)
- element registered as a narrow **nullable** byte Parts (stride 1, not the wide 8)
- write (append): `OpSetByteNullable(_elm, 0, 0, <val | OpConvIntFromNull()>)`
- read (index):   `OpGetByteNullable(OpGetVectorNullable(v, 1, i), 0, 0)`
- `v[0] == null` → true; `v[0]` prints `null`; `v[1]` (=42) prints 42, not null.

## No-regression predictions (must stay UNCHANGED)
- `vector<u8>` (non-null): write `OpSetByte`, read `OpGetByte(GetVectorNullable(v,1,i),0,0)`; holds 255.
- `vector<boolean?>` / `vector<character?>`: unchanged (already hold null).
- `u8?` / `i8?` / `u16?` FIELD read+write: byte-identical.

## Root cause (5 coupled sites, one real fact missing)
The `narrow_vec` flag was a proxy for "raw, no sentinel", neutralized everywhere by
`&& !narrow_vec`. The real fact — the element is DECLARED nullable (`Optional`) so its
slot reserves a sentinel — never reached `NarrowIntKind::of`. Fix threads that fact:
1. `data.rs narrow_vector_content` — peel `Optional`, return the nullable narrow Parts.
2. `data.rs NarrowIntKind::of` — a nullable narrow-vec byte/short is `ByteNullable`/`Short`
   (sentinel), not raw; add `reserves_sentinel()`.
3. `fields.rs` index read — pass the element's declared nullability (`Optional`?) to
   `get_val`, not the hardcoded OOB `true`.
4. `mod.rs get_val` + `set_field_check` — derive the sentinel `min` from the kind
   (`kind.reserves_sentinel()`), not `nullable && !narrow_vec`.
5. `vectors.rs new_record` — route the narrow element write through `NarrowIntKind`
   (peel `Optional`) so the append op == the index-read op for every width.
