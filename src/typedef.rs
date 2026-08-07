// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I59 — Type resolver

//! Type resolution and field layout.
//!
//! After the parser's first pass declares all types, this module resolves
//! forward references, computes field sizes and offsets, and initialises
//! database store schemas.  Called between parser pass 1 and pass 2.
//!
//! Key entry points:
//! - [`actual_types`] — resolve forward type references, detect cycles,
//!   compute field positions via [`crate::calc::calculate_positions`].
//! - [`fill_all`] — allocate database stores for each struct/enum and
//!   write the type schema into `Stores`.
//! - [`complete_definition`] — finalise a single definition's field layout.

use crate::data::{Data, DefType, Deps, I32, IntegerSpec, Type, Value};
use crate::database::Stores;
use crate::diagnostics::Level;
use crate::lexer::Lexer;

/// Set the correct type and initial size in definitions.
/// This will not factor in the space for attributes for records
/// as we still need to analyze the actual use of records.
pub fn complete_definition(_lexer: &mut Lexer, data: &mut Data, d_nr: u32) {
    match data.def(d_nr).name.as_str() {
        "vector" => {
            data.set_returned(d_nr, Type::Vector(Box::new(Type::Unknown(0)), Deps::none()));
            data.definitions[d_nr as usize].known_type = 7;
        }
        "integer" => {
            data.set_returned(d_nr, I32.clone());
            data.definitions[d_nr as usize].known_type = 0;
        }
        "float" => {
            data.set_returned(d_nr, Type::Float);
            data.definitions[d_nr as usize].known_type = 3;
        }
        "single" => {
            data.set_returned(d_nr, Type::Single);
            data.definitions[d_nr as usize].known_type = 2;
        }
        "text" => {
            data.set_returned(d_nr, Type::Text(Deps::none()));
            data.definitions[d_nr as usize].known_type = 5;
        }
        "boolean" => {
            data.set_returned(d_nr, Type::Boolean);
            data.definitions[d_nr as usize].known_type = 4;
        }
        "enumerate" => {
            data.set_returned(d_nr, Type::Enum(0, false, Deps::none()));
        }
        "function" => {
            data.set_returned(d_nr, Type::Routine(d_nr));
        }
        "character" => {
            data.set_returned(d_nr, Type::Character);
            data.definitions[d_nr as usize].known_type = 6;
        }
        "radix" | "hash" | "reference" | "index" | "sorted" | "spatial" | "trie" => {
            data.set_returned(d_nr, Type::Reference(d_nr, Deps::none()));
        }
        "keys_definition" => {
            data.set_returned(d_nr, Type::Keys);
            data.definitions[d_nr as usize].known_type = 8;
        }
        _ => {}
    }
}

/// loft#417 / @PLN25 — the ONE home for "is this `vector<S>` struct-element the
/// synthetic `__nullable<S>` enum (the nullable-by-default), and which def is it?".
/// BOTH the parse-time chokepoint (`e2_nullable_elem`) and the deferred forward-ref
/// resolver (`copy_unknown_fields` below) call it, so a `vector<S>` element resolves
/// to the SAME type whether `S` was defined before its use (known → rewritten at
/// parse) or after it (forward-ref → still `Unknown` at parse, resolved here once `S`
/// is finally known).  Before this single home existed the two resolvers disagreed:
/// the field went dense while its params/locals went nullable → element-stride
/// mismatch → corrupted enum-discriminant reads on both backends (loft#417).  Returns
/// the synth `__nullable<S>` enum def for an eligible element (a non-stdlib,
/// non-synthetic struct); `None` leaves the element dense.
pub(crate) fn nullable_vector_elem(
    data: &mut Data,
    lexer: &mut Lexer,
    struct_d: u32,
) -> Option<u32> {
    if struct_d == u32::MAX
        || !matches!(data.def_type(struct_d), DefType::Struct)
        || data.def(struct_d).synthetic.is_some()
        || data.def(struct_d).source == crate::data::STD_SOURCE
    {
        return None;
    }
    Some(data.nullable_enum_for(lexer, struct_d))
}

fn copy_unknown_fields(data: &mut Data, d: u32) {
    for nr in 0..data.attributes(d) {
        // `Unknown(was)` names the forward-referenced type's STUB def — except for
        // `Unknown(0)`, which is the codebase-wide "no type known" sentinel and names
        // nothing (`Type::Unknown(0)` is what every unresolved expression carries).
        // Resolving THAT against definition #0 hands the field whatever the first
        // definition in the program happens to return — `text`, in practice — so a
        // field with no type silently acquires a plausible wrong one instead of staying
        // visibly unresolved (#686).  The `Vector` arm below already guarded it; this is
        // the same guard on the bare case.
        //
        // A `?` on the field is transparent here: `S?` and `S` name the same
        // forward reference, so peel the marker, resolve, and put it back.  Without
        // that, a `Roofs?` field kept `Optional(Unknown(stub))` after every other
        // spelling had resolved, and the first read of it reported the internal type
        // name (`optional(unknown(700))`) at the CALLER (loft#797).
        let (attr_type, optional) = match data.attr_type(d, nr) {
            Type::Optional(inner) => (*inner, true),
            other => (other, false),
        };
        if let Type::Unknown(was) = attr_type
            && was != 0
        {
            let resolved = data.def(was).returned.clone();
            set_attr_type_keeping_optional(data, d, nr, resolved, optional);
        } else if let Type::Vector(content, dep) = &attr_type
            && let Type::Unknown(was) = **content
            && was != 0
        {
            let dep = dep.clone();
            // Forward-ref element resolves DENSE — the dense-default invariant
            // ("`vector<τ>` is dense for every τ unless an explicit `τ?`").  A
            // `vector<S?>` carries its `?` from parse as a synth `__nullable<S>`
            // enum element (`e2_nullable_elem` registers it eagerly even when S is
            // a forward ref), so only the dense `vector<S>` ever reaches here as a
            // bare `Unknown`.  Wrapping it here (the pre-dense behaviour) is what
            // made a forward-referenced element's FIELD nullable while its
            // construction stayed dense → element-stride mismatch → corrupted
            // enum-discriminant reads / over-free on both backends (@PLN25 #465).
            let resolved = data.def(was).returned.clone();
            set_attr_type_keeping_optional(
                data,
                d,
                nr,
                Type::Vector(Box::new(resolved), dep),
                optional,
            );
        }
    }
}

/// Write a freshly resolved type onto attribute `nr`, restoring the `?` the caller peeled.
///
/// `set_attr_type` guards against overwriting a type that is already settled, and it reads
/// "settled" as `is_unknown() == false` — which an `Optional(…)` wrapper always is, whatever
/// it wraps.  Re-wrapping before the call would therefore trip the guard on exactly the
/// resolution it exists to allow, so the optional case writes the field directly, the same
/// escape [`Data::rewrite_unknown_refs`] takes for `Vector<Unknown>` and friends.
fn set_attr_type_keeping_optional(
    data: &mut Data,
    d: u32,
    nr: usize,
    resolved: Type,
    optional: bool,
) {
    if optional {
        data.definitions[d as usize].attributes[nr].typedef = Type::Optional(Box::new(resolved));
    } else {
        data.set_attr_type(d, nr, resolved);
    }
}

/// Resolve forward type references accumulated during parsing.  When
/// `defer_unknown` is `Some`, every `DefType::Unknown` stub is recorded
/// as `(source, def_nr, position)` in the passed-in vec instead of being
/// emitted as a diagnostic — the caller is then responsible for either
/// patching the stub (via `Data::rewrite_unknown_refs`) or surfacing the
/// final "Undefined type" error later.
///
/// The package-mode driver uses this: cyclic intra-package `use`
/// declarations legitimately produce Unknown stubs for cross-file types
/// that will be resolved by `resolve_deferred_unknowns` after both sides
/// of the cycle have registered their definitions.
pub fn actual_types_deferred(
    data: &mut Data,
    database: &mut Stores,
    lexer: &mut Lexer,
    start_def: u32,
    mut defer_unknown: Option<&mut Vec<(u16, u32, crate::data::Position)>>,
) {
    // Determine the actual type of structs regarding their use
    for d in start_def..data.definitions() {
        if matches!(data.def_type(d), DefType::Struct) {
            data.definitions[d as usize].returned = Type::Reference(d, Deps::none());
        }
    }
    for d in start_def..data.definitions() {
        match data.def_type(d) {
            DefType::Unknown => {
                if let Some(buf) = defer_unknown.as_deref_mut() {
                    let def = data.def(d);
                    buf.push((def.source, d, def.position.clone()));
                    continue;
                }
                let name = &data.def(d).name;
                // `string` used to be special-cased here; it is now one row of the
                // cross-language alias table `suggest_type_name` consults, so this
                // site has one path and the table has one home (Goal E).
                let msg = if let Some(s) = data.suggest_type_name(name) {
                    format!("Undefined type {name} — did you mean '{s}'?")
                } else {
                    format!("Undefined type {name}")
                };
                lexer.pos_diagnostic(Level::Error, &data.def(d).position, &msg);
            }
            DefType::Function => {
                copy_unknown_fields(data, d);
                if let Type::Unknown(was) = data.def(d).returned {
                    data.set_returned(d, data.def(was).returned.clone());
                }
            }
            DefType::Struct => {
                copy_unknown_fields(data, d);
            }
            DefType::Enum => {
                // @PLN25 — a synthetic `__nullable<S>` enum's `Some` variant carries
                // S's fields, so its DB layout depends on S already being resolved.
                // Registering it HERE (before the struct layout loop) `enumerate`s
                // it at an index whose size is computed before S, so in a multi-type
                // program the enum's inline size is wrong (disc-only, 8 B) and
                // `vector<__nullable<S>>` construction/reads use the wrong stride.
                // Leave these to `synth_nullable_struct_fields` in `fill_all`, which
                // runs AFTER the struct-resolution order is established.
                if !(data.def(d).synthetic.is_some() && data.def(d).name.starts_with("__nullable<"))
                {
                    register_enum_db(data, database, d);
                }
            }
            DefType::EnumValue if data.attributes(d) > 0 => {
                copy_unknown_fields(data, d);
            }
            _ => {}
        }
    }
}

/// #682 — carry the post-`scopes::check` capture-ownership verdict into the
/// already-registered schema, so `free_named`'s cascade frees exactly the
/// captures the closure record adopted.
///
/// The interpreter's schema is laid out during parse, but which captures a record
/// owns is only settled by scope analysis (`scopes::mark_borrowed_captures`) —
/// hence this second, layout-preserving pass rather than a decision inside
/// `fill_database`.  `--native` needs no equivalent: its schema is emitted from
/// these same attribute types AFTER scope analysis has run.
///
/// Idempotent, and safe to run before scope analysis has marked anything (it then
/// finds no borrowed attribute and changes nothing).
pub fn sync_capture_ownership(data: &Data, database: &mut Stores) {
    for d in 0..data.definitions() {
        if !data.def(d).name.starts_with("__closure_") {
            continue;
        }
        let known = data.def(d).known_type();
        if known == u16::MAX {
            continue;
        }
        for a in 0..data.attributes(d) {
            if matches!(data.attr_type(d, a), Type::Reference(_, ref deps) if deps.is_borrowed_share())
            {
                database.borrow_dbref_field(known, &data.attr_name(d, a));
            }
        }
    }
}

/// Whether `d`'s layout has to wait, because a field's type is not known yet.
///
/// Laying a struct out anyway is what makes the failure a CORRUPTION rather than a
/// refusal: the field loop in [`fill_database`] silently skips an attribute it cannot
/// size, but the type is still registered and `finish` still sizes it — so the field
/// keeps `position == u16::MAX` forever, and `finish_type` will not revisit an
/// already-sized type.  The declaration and the layout then disagree for the rest of
/// the run: #686's closure body read and wrote its capture at offset 65535, and #797's
/// package field did the same to its record's neighbours.
///
/// Deferring costs nothing.  The registration loop in [`fill_all`] is keyed on
/// `known_type == u16::MAX`, so the next `fill_all` — the next file's, or the one after
/// `resolve_deferred_unknowns` — picks the struct up as soon as the type arrives.  A
/// field that never resolves leaves the struct unregistered, which is harmless: the
/// parser has already reported the undefined type, and `field_position` names the field
/// if anything still reaches for it.
///
/// Two kinds of "not known yet" reach here, and both must block:
///
///  * `Unknown(0)` names nothing.  It is what a field typed from an EXPRESSION carries,
///    and the only producer is the closure record — a capture's type is the type of
///    `w.chunks[1]`, not of a written-down name.  `resolve_forward_captures` repairs it.
///  * `Unknown(stub)` names a forward-referenced type whose declaration has not parsed
///    yet.  Within one file `copy_unknown_fields` resolves it before the layout loop, but
///    it only sweeps the file being finished — a struct in a module the package loaded
///    EARLIER keeps its stub, because the type it names is declared by a module still
///    suspended further up the `use` chain (loft#797).  The sweep at the top of
///    `fill_all` re-resolves those; whatever is still `Unknown` here is genuinely unknown.
///
/// The answer is transitive.  An inline struct field stores its content's bytes, so a
/// host whose field type is itself waiting cannot be laid out either — laying it out
/// would register the field with content id `u16::MAX`.
fn layout_blocked(data: &Data, d: u32, seen: &mut Vec<u32>) -> bool {
    if d == u32::MAX {
        return true;
    }
    if matches!(data.def_type(d), DefType::Unknown) {
        return true;
    }
    if data.def(d).known_type != u16::MAX {
        return false; // already laid out — its fields were known then
    }
    if !matches!(data.def_type(d), DefType::Struct | DefType::EnumValue) {
        return false;
    }
    if seen.contains(&d) {
        // A value cycle is rejected by `fill_all`'s own check; stopping here just
        // keeps this walk finite.
        return false;
    }
    seen.push(d);
    let blocked = (0..data.attributes(d)).any(|a| {
        !data.def(d).attributes[a].constant && type_blocked(data, &data.attr_type(d, a), seen)
    });
    seen.pop();
    blocked
}

/// The [`layout_blocked`] question asked of a TYPE rather than a definition: does laying
/// this out need a size nobody can supply yet?
///
/// The set of forms mirrors what [`fill_database`] actually asks of a field's content, so
/// that the two agree on which fields have a dependency at all.  A keyed collection needs
/// its content's type ID; a `Reference` with EMPTY deps stores the content's bytes inline
/// and so needs its size — but one with deps is a fixed-width `DbRef`, sized whatever the
/// content turns out to be, which is why that case does not wait (the same split the
/// native generator's field-hoist makes).
fn type_blocked(data: &Data, tp: &Type, seen: &mut Vec<u32>) -> bool {
    match tp.base() {
        Type::Unknown(_) => true,
        Type::Vector(c, _) | Type::RefVar(c) => type_blocked(data, c, seen),
        Type::Tuple(elms) => elms.iter().any(|e| type_blocked(data, e, seen)),
        Type::Hash(c, _, _)
        | Type::Index(c, _, _)
        | Type::Sorted(c, _, _)
        | Type::Radix(c, _, _)
        | Type::Trie(c, _, _) => layout_blocked(data, *c, seen),
        Type::Reference(c, deps) if deps.is_empty() => layout_blocked(data, *c, seen),
        _ => false,
    }
}

pub fn fill_all(data: &mut Data, database: &mut Stores, lexer: &mut Lexer, start_def: u32) {
    // Re-resolve the forward references of everything still waiting for a layout.
    //
    // `actual_types_deferred` sweeps only the file it is finishing, so a struct that
    // named a not-yet-declared type keeps `Unknown(stub)` on the attribute after its own
    // file is done.  The stub def is upgraded IN PLACE the moment its real declaration
    // parses (`parse_struct` reuses the stub's def_nr), which makes the attribute
    // resolvable from here on — but nothing was asking again.  That is loft#797: the
    // declaration ended up correct and the layout kept the hole.
    //
    // Ask again for every def whose layout is still pending, so each `fill_all` picks up
    // whatever the files parsed since have declared.  Resolving against a def that is
    // still a stub is a no-op (a stub's `returned` is its own `Unknown`), so this
    // converges without needing to know which pass finally supplies the type.
    for d_nr in 0..data.definitions() {
        if data.def(d_nr).known_type == u16::MAX
            && matches!(data.def_type(d_nr), DefType::Struct | DefType::EnumValue)
        {
            copy_unknown_fields(data, d_nr);
        }
    }
    // Detect type cycles before computing sizes.
    for d_nr in start_def..data.definitions() {
        if matches!(data.def_type(d_nr), DefType::Struct) {
            let mut visiting = std::collections::HashSet::new();
            if data.has_value_cycle(d_nr, &mut visiting) {
                lexer.pos_diagnostic(
                    Level::Error,
                    &data.def(d_nr).position,
                    &format!(
                        "Struct '{}' contains itself (directly or indirectly) — use reference<{}> to break the cycle",
                        data.def(d_nr).name,
                        data.def(d_nr).name,
                    ),
                );
            }
        }
    }
    // reject hash-value structs that have a field named `key`.
    // `key` is a reserved pseudo-field for hash iteration (`for kv in h { kv.key }`).
    for d_nr in start_def..data.definitions() {
        if !matches!(data.def_type(d_nr), DefType::Struct) {
            continue;
        }
        for a_nr in 0..data.attributes(d_nr) {
            if let Type::Hash(c_nr, _, _) = data.attr_type(d_nr, a_nr)
                && data.attr(c_nr, "key") != usize::MAX
            {
                lexer.pos_diagnostic(
                    Level::Error,
                    &data.def(c_nr).position,
                    &format!(
                        "Struct '{}' has a field named 'key' which is reserved for hash iteration — rename the field",
                        data.def(c_nr).name,
                    ),
                );
            }
        }
    }
    // @PLN25 E2 — register the synthetic `__nullable<T>` enums + (gated) rewrite
    // embedded struct fields, BEFORE the unit-variant discriminant pass + the
    // layout loop below, so each synthetic enum is registered and laid out like
    // any hand-written enum.  Called unconditionally: the registration sweep
    // inside is a no-op when no `__nullable<>` enum exists (gate off), and the
    // field-rewrite arm is gated internally.
    synth_nullable_struct_fields(data, database, lexer);
    // Start from 0 (not start_def) so struct-enum variants defined in earlier
    // default library files are processed when later files trigger fill_all.
    // The has_type guard prevents double-processing.  Fixes S14 (PROBLEMS #80).
    // B2-runtime (2026-04-13): Before laying out records, retroactively
    // add a discriminant "enum" field to every unit variant of a mixed
    // struct-enum.  `parse_enum_values` only adds this field inside the
    // `has_token("{")` branch (struct variants), so sibling unit variants
    // have 0 attributes and would produce a size-0 structure — runtime
    // `OpDatabase(db_tp=…)` then panics `Incomplete record` in
    // `Store::claim(size=0)`.  Check the parent's `returned` (set to
    // `Type::Enum(_, true, _)` when ANY variant has braces) rather than
    // the unit-variant child's (which stays `Type::Enum(_, false, _)`).
    let enumerate_d_nr = data.def_nr("enumerate");
    if enumerate_d_nr != u32::MAX {
        for d_nr in 0..data.definitions() {
            if matches!(data.def_type(d_nr), DefType::EnumValue) && data.attributes(d_nr) == 0 {
                let parent = data.def(d_nr).parent;
                if parent != u32::MAX && matches!(data.def(parent).returned, Type::Enum(_, true, _))
                {
                    let discriminant = {
                        let mut v: u8 = 0;
                        for (a_nr, a) in data.def(parent).attributes.iter().enumerate() {
                            if a.name == data.def(d_nr).name {
                                v = a_nr as u8 + 1;
                                break;
                            }
                        }
                        v
                    };
                    data.add_attribute(
                        lexer,
                        d_nr,
                        "enum",
                        Type::Enum(enumerate_d_nr, false, Deps::none()),
                    );
                    let attr_nr = data.def(d_nr).attr_names["enum"];
                    data.set_attr_value(d_nr, attr_nr, Value::Enum(discriminant, u16::MAX));
                }
            }
        }
    }
    // QUALITY B5 fix: register `main_vector<T>` wrapper structs for every
    // `vector<T>` field found on a struct or enum-value.  Parser paths
    // that assign or construct a `vector<T>` already call
    // `data.vector_def(...)`, but **struct-enum variant fields** (e.g.
    // `Node { kids: vector<Tree> }` inside `enum Tree`) go through
    // `parse_enum_values` / `fill_all` without ever hitting a vector
    // assignment site.  Without the wrapper, `gen_set_first_vector_null`'s
    // `data.name_type("main_vector<Tree>")` lookup returns `u16::MAX`
    // and the interpreter emits `OpDatabase(var, db_tp=u16::MAX)` that
    // panics in `Store::claim` as "Incomplete record".  Register the
    // wrappers here, BEFORE the main `fill_database` loop, so the loop
    // then picks them up and assigns a real `known_type`.
    let mut pending: Vec<Type> = Vec::new();
    for d_nr in 0..data.definitions() {
        if !(matches!(data.def_type(d_nr), DefType::Struct)
            || matches!(data.def_type(d_nr), DefType::EnumValue))
        {
            continue;
        }
        for a_nr in 0..data.attributes(d_nr) {
            if let Type::Vector(content, _) = data.attr_type(d_nr, a_nr) {
                let content_tp = *content;
                let wrapper_name = format!("main_vector<{}>", content_tp.name(data));
                if data.def_nr(&wrapper_name) == u32::MAX {
                    pending.push(content_tp);
                }
            }
        }
    }
    for tp in pending {
        data.vector_def(lexer, &tp);
    }
    for d_nr in 0..data.definitions() {
        // @PLN22 Phase 2 — register every not-yet-registered struct / struct-enum
        // variant.  The guard is PER-DEF (`known_type == u16::MAX`), not
        // per-bare-name: a second def with a name the stdlib/another source
        // already registered (a shadowing `struct File`, or P379's two-library
        // same-name structs) must still be filled — fill_database registers it
        // under a source-qualified name.  A bare-name guard skipped it, leaving
        // `known_type = u16::MAX` and a runtime out-of-bounds on `self.types`.
        if ((matches!(data.def_type(d_nr), DefType::EnumValue) && data.attributes(d_nr) > 0)
            || matches!(data.def_type(d_nr), DefType::Struct))
            && data.def(d_nr).known_type == u16::MAX
            && !layout_blocked(data, d_nr, &mut Vec::new())
        {
            fill_database(data, database, d_nr);
            // @PLN25 E2 — right after building a struct `S`, build its synthetic
            // `__nullable<S>` enum's `Null` + `Some` variant STRUCTURES (if that
            // enum exists), so they take type-ids that follow `S` (and its field
            // types) but PRECEDE any later struct that holds a `hash<__nullable<S>>`
            // field.  Native codegen creates a keyed-collection struct field INLINE
            // and relies on its tid being reachable when the struct emits; building
            // the variants lazily (during that struct's hash field) gives `Some` a
            // tid AFTER the struct, so native interns hash<->Some swapped and a
            // baked `OpGetRecord(hash_tid)` resolves to `Some` → `find called on
            // non-collection type` at runtime.  Building them here (after `S`, not
            // up-front) keeps `S`'s field types created first (no native forward-
            // ref) yet still ahead of consumers.  Gate-inert: `__nullable<>` enums
            // exist only gate-on.
            if matches!(data.def_type(d_nr), DefType::Struct) {
                let syn_name = format!("__nullable<{}>", data.def(d_nr).name());
                let syn = data.def_nr(&syn_name);
                if syn != u32::MAX && data.def(syn).known_type != u16::MAX {
                    for variant in ["Null", "Some"] {
                        let v = data.variant_of(syn, variant);
                        if v != u32::MAX && data.def(v).known_type == u16::MAX {
                            fill_database(data, database, v);
                        }
                    }
                }
            }
        }
    }
    // P191 — pre-register database types for local-var keyed
    // collections (index/hash/spatial) so their bookkeeping fields
    // get appended to the content struct BEFORE database.finish()
    // runs finish_type to assign positions.
    //
    // Only Index appends bookkeeping fields (#left/#right/#color)
    // to the content struct; Hash/Radix just create an entry in
    // self.types without struct mutation.  But registering all three
    // here keeps the codepath uniform with what gen_set_first_keyed_null
    // would do later — and is idempotent (database.{index,hash,spatial}
    // dedup on name).
    //
    // Sorted is NOT in this loop — sorted doesn't append bookkeeping
    // fields, and P190's on-demand registration in get_type already
    // handles it.  Adding Sorted here would be a no-op anyway.
    //
    // **Critical timing**: this runs at the end of fill_all, which
    // runs at the end of EACH parse_file call.  At end of first-pass
    // parse_file, function variables are populated by parse_code (line
    // 804 of definitions.rs, called in both passes).  So the registration
    // happens BEFORE second-pass body parsing, which means
    // database.position() lookups during second-pass IR construction
    // see the post-bookkeeping struct layout.  Without this timing,
    // bookkeeping fields appended later (by gen_set_first_keyed_null
    // at codegen) stay at position 0 because finish_type only runs
    // for types with size == u16::MAX.
    for d_nr in start_def..data.definitions() {
        if !matches!(data.def_type(d_nr), DefType::Function) {
            continue;
        }
        let var_count = data.def(d_nr).variables.count();
        for v in 0..var_count {
            let tp = data.def(d_nr).variables.tp(v).clone();
            match tp {
                Type::Hash(c, key, _) => {
                    let c_tp = data.def(c).known_type;
                    if c_tp != u16::MAX {
                        database.hash(c_tp, &key);
                    }
                }
                Type::Index(c, key, _) => {
                    let c_tp = data.def(c).known_type;
                    if c_tp != u16::MAX {
                        database.index(c_tp, &key);
                    }
                }
                Type::Radix(c, key, _) => {
                    let c_tp = data.def(c).known_type;
                    if c_tp != u16::MAX {
                        database.spatial(c_tp, &key);
                    }
                }
                Type::Trie(c, key, _) => {
                    let c_tp = data.def(c).known_type;
                    if c_tp != u16::MAX {
                        database.trie(c_tp, &key);
                    }
                }
                _ => {}
            }
        }
    }
}

/// @PLN25 E2a.2 — rewrite each nullable struct-typed field to the synthetic
/// `__nullable<T>` enum (`Null | Some<fields>`), so an absent value is
/// representable inline (discriminant `0`) instead of crashing the
/// `OpCopyRecord`-of-a-null-source path.  Runs at the very start of `fill_all`
/// (before the unit-variant discriminant pass + the layout loop) so the
/// synthetic enum is registered (`register_enum_db`) and laid out like any
/// hand-written enum.
///
/// Two arms with DIFFERENT maturity (called unconditionally; each arm gated):
/// - **Embedded-field rewrite** (`item: Row` → `__nullable<Row>`) — gated on
///   `LOFT_E2_FIELDS`, non-stdlib only.  Immature: a plain `b.item.id` read does
///   not auto-unwrap the enum, so flipping struct fields tree-wide breaks field
///   reads across the stdlib + libraries.
/// - **Registration sweep** — unconditional; lays out every `__nullable<S>`
///   enum the VECTOR-element path (`e2_nullable_elem`, gated on `LOFT_E2_SYNTH`)
///   created at parse time.  A no-op when none exist (gate off).
fn synth_nullable_struct_fields(data: &mut Data, database: &mut Stores, lexer: &mut Lexer) {
    // @PLN25 — embedded NON-vector struct-field nullability (`item: Row` →
    // `__nullable<Row>`) is DEFERRED behind an opt-in (`LOFT_E2_FIELDS`): unlike
    // the nullable-SEQUENCE work (`vector<S>` elements, shipped default-on), the
    // field access/construct glue is incomplete — a plain `b.item.id` read does
    // not auto-unwrap the enum, so flipping every struct field tree-wide breaks
    // field reads across the stdlib + libraries.  The registration loop below
    // (which lays out the `__nullable<S>` enums the VECTOR path creates) always
    // runs.  Trigger to lift this: the field read/construct auto-unwrap glue.
    if std::env::var("LOFT_E2_FIELDS").is_ok() {
        for host in 0..data.definitions() {
            // Stdlib stays dense (this arm is immature); synthetic hosts (tuples,
            // fn-ref, and our own `__nullable<T>` variants) are skipped so the
            // rewrite never recurses into generated layouts.
            if data.def(host).source == crate::data::STD_SOURCE
                || data.def(host).synthetic.is_some()
            {
                continue;
            }
            if !(matches!(data.def_type(host), DefType::Struct)
                || (matches!(data.def_type(host), DefType::EnumValue) && data.attributes(host) > 0))
            {
                continue;
            }
            for a_nr in 0..data.attributes(host) {
                // Skip the per-variant `constant` markers and `not null` fields.
                if data.def(host).attributes[a_nr].constant || !data.attr_nullable(host, a_nr) {
                    continue;
                }
                // Rewrite an EMBEDDED non-vector struct field `item: Row` → the
                // synthetic `__nullable<Row>` enum.  A field VECTOR `items:
                // vector<Row>` is rewritten at the vector-type chokepoint
                // (`sub_type` `vector` arm), so by here its content is already
                // the enum — not a `Reference` — and falls through.  Keyed
                // collections, primitives, fn-refs out of scope.
                let Type::Reference(struct_d, _) = data.attr_type(host, a_nr) else {
                    continue;
                };
                if data.def_type(struct_d) != DefType::Struct
                    || data.def(struct_d).synthetic.is_some()
                {
                    continue;
                }
                let syn = data.nullable_enum_for(lexer, struct_d);
                if data.def(syn).known_type == u16::MAX {
                    register_enum_db(data, database, syn);
                }
                data.definitions[host as usize].attributes[a_nr].typedef =
                    Type::Enum(syn, true, Deps::none());
            }
        }
    }
    // E2a.5b — register any synthetic `__nullable<>` enum the LOCAL/param
    // parse-time rewrite (expressions.rs `e2_nullable_vec_local`) created but the
    // field loop above did not reach (no struct field references it).  Doing the
    // `register_enum_db` HERE — in `fill_all`, before the layout loop — instead of
    // mid-body-parse is what keeps the discriminant db-type laid out correctly;
    // registering it during parsing corrupts every read of the shared enum.
    for d in 0..data.definitions() {
        if matches!(data.def_type(d), DefType::Enum)
            && data.def(d).synthetic.is_some()
            && data.def(d).known_type == u16::MAX
            && data.def(d).name.starts_with("__nullable<")
        {
            register_enum_db(data, database, d);
        }
    }
}

/// Register an enum's database type and its variant entries, and stamp each
/// parent variant attribute's discriminant value with the database enum id.
/// Shared by `actual_types_deferred` (hand-written enums) and `fill_all`'s
/// @PLN25 nullable-struct-field synthesis (synthetic `__nullable<T>` enums),
/// so both register identically.
fn register_enum_db(data: &mut Data, database: &mut Stores, d: u32) {
    let mut name = data.def(d).name.clone();
    // @PLN22 (p379 `two_libs_same_struct_name`): two libraries may each define a struct
    // of the same name `S`.  `nullable_enum_for` already gives each `S` its own synth
    // `__nullable<S>` DEF (keyed on the struct's source), but both DEFS share the bare db
    // name `__nullable<S>`, so their `Null`/`Some` variant structures collide in the flat
    // db type table ("Double structure type __nullable<S>::Null") and field access binds
    // to the wrong payload struct.  When the bare name is already a db type (the second
    // definer), disambiguate by the PAYLOAD struct's QUALIFIED def name — keeping the
    // `__nullable<` prefix so `nullable_some_variant` still resolves it (a `lib::`-prefixed
    // `qualified_type_name` would not).  The struct's db name is not laid out yet at this
    // point (register runs before the struct layout loop), so use the def name, not the db
    // name.  The first definer keeps the bare name, so non-colliding programs are unchanged.
    if data.def(d).synthetic.is_some()
        && name.starts_with("__nullable<")
        && database.has_type(&name)
    {
        let some_v = data.variant_of(d, "Some");
        let payload_attr = data.attr(some_v, "payload");
        if payload_attr != usize::MAX
            && let Type::Reference(sd, _) = data.attr_type(some_v, payload_attr)
        {
            name = format!("__nullable<{}>", data.qualified_type_name(sd));
        }
    }
    let e_nr = database.enumerate(&name);
    for a in 0..data.attributes(d) {
        database.value(e_nr, &data.attr_name(d, a), u16::MAX);
        data.set_attr_value(d, a, Value::Enum(a as u8 + 1, e_nr));
    }
    data.definitions[d as usize].known_type = e_nr;
}

/// @PLN25 — register + lay out a synth `__nullable<S>` enum ON DEMAND, for a FORWARD-referenced `S`
/// whose synth enum is first created during pass-2 body parse — AFTER `fill_all`'s in-order
/// registration ran.  Without it the enum keeps `known_type == u16::MAX`, the `Some` payload byte
/// position + the element size both read 0/MAX, and `v[0].field` reads garbage (371).  `S` is fully
/// laid out by pass 2, so this sizes the enum + `Some` immediately.  No-op once registered.
///
/// CRITICAL: do NOT `fill_database` the ENUM def itself — that runs `structure()` and re-registers it
/// (under a qualified name), OVERWRITING `known_type` away from the enum, so the variant link below
/// targets a struct and the `Some` size never reaches `Parts::Enum`.  Only the VARIANT structs go
/// through `fill_database` (its `EnumValue` arm calls `enum_value` to link each into the enum).
// @PLN25 dense flip — superseded for the paths that exist today by
// `nullable_vector_elem` + `copy_unknown_fields` (e2_nullable_elem), which handle the
// forward-referenced synth layout. Kept (allow dead) for the Phase-0 EXPAND residual:
// when `?` parsing extends to return/param type positions, a forward-ref `vector<S?>`
// synth created there may need this on-demand layout again.
#[allow(dead_code)]
pub(crate) fn register_and_lay_out_synth(data: &mut Data, database: &mut Stores, synth_d: u32) {
    if synth_d == u32::MAX || data.def(synth_d).known_type() != u16::MAX {
        return;
    }
    register_enum_db(data, database, synth_d);
    let variants: Vec<u32> = data.children_of(synth_d).collect();
    for v in &variants {
        fill_database(data, database, *v);
    }
    let enum_kt = data.def(synth_d).known_type();
    let some_d = data.variant_of(synth_d, "Some");
    let some_kt = if some_d == u32::MAX {
        u16::MAX
    } else {
        data.def(some_d).known_type()
    };
    database.lay_out_synth(enum_kt, some_kt);
}

/// A free DB structure name for an enum VARIANT whose bare and source-qualified
/// names are both taken — `<parent enum's DB name>::<variant>`.
///
/// The parent enum is itself a registered structure, so its DB name is already
/// unique and the variant name below it cannot collide. `None` when the def is not
/// a variant, when the parent has no registered type yet, or (defensively) when
/// even that name is taken — the caller then keeps the source-qualified name and
/// the registration aborts with its own diagnostic rather than a silent alias.
fn variant_parent_qualified_name(data: &Data, database: &Stores, d_nr: u32) -> Option<String> {
    if data.def_type(d_nr) != DefType::EnumValue {
        return None;
    }
    let parent = data.def(d_nr).parent;
    let parent_name = database.type_name(data.def(parent).known_type);
    if parent_name.is_empty() {
        return None;
    }
    let name = format!("{parent_name}::{}", data.def(d_nr).name);
    (!database.has_type(&name)).then_some(name)
}

pub(crate) fn fill_database(data: &mut Data, database: &mut Stores, d_nr: u32) {
    if data.def(d_nr).name == "Unknown(0)" {
        return;
    }
    // The generic type-var marker (`<T>`) is a single shared def referenced by every
    // `vector<T>` param across the stdlib generics; fill it ONCE.  A second fill for
    // another such param must be a no-op or `database.structure` would panic on the
    // mangled name below.
    if data.is_type_var_placeholder(d_nr) && data.def(d_nr).known_type != u16::MAX {
        return;
    }
    let mut enum_value = 0;
    if let Type::Enum(nr, true, _) = data.def(d_nr).returned {
        for (a_nr, a) in data.def(nr).attributes.iter().enumerate() {
            if a.name == data.def(d_nr).name {
                enum_value = a_nr as i32 + 1;
                break;
            }
        }
    }
    // @P379 — struct-type registration is a flat table keyed by name.  When
    // two libraries each define a struct of the same bare name (different
    // field layouts), register the second under a library-qualified name
    // (`moros_map::Chunk`) instead of panicking `Double structure type`.
    // The bare name stays for the first/only definer, so non-colliding
    // programs are byte-identical.  The parser already resolves each usage
    // to the correct per-library `d_nr` (and hence `known_type`); this only
    // makes the database table tolerate the shared bare name.
    // @PLN25 — synthetic `__nullable<S>` enums all name their variants `Null` /
    // `Some`, but `Some` carries a DIFFERENT payload per `S`, so they cannot share
    // one structure-table entry (and `Null` would `Double structure type` the
    // moment a second `__nullable<>` enum exists).  Register each under its
    // parent-enum-qualified name (`__nullable<Row>::Some`) so the flat DB type
    // table stays collision-free.  Variant lookup keys on the bare name + parent
    // enum (`database.enum_value` below) and runtime discriminants, so this
    // changes only the structure-table key, not resolution.
    // @PLN22 (p379 `two_libs_same_struct_name`): two libs may define same-named structs `S`, so a
    // synth `__nullable<S>`'s `Null`/`Some` variant structures need a UNIQUE db name.  Key the
    // variant on the PARENT enum's DB name (`register_enum_db` already disambiguated it — bare
    // `__nullable<S>` for the first definer, `__nullable<lib::S>` for the second), NOT the parent's
    // bare DEF name (shared by both), so the two libs' `__nullable<S>::Null` no longer collide.
    let synth_variant_name = if data.def_type(d_nr) == DefType::EnumValue {
        let parent = data.def(d_nr).parent;
        (data.def(parent).synthetic.is_some() && data.def(parent).name.starts_with("__nullable<"))
            .then(|| {
                format!(
                    "{}::{}",
                    database.type_name(data.def(parent).known_type),
                    data.def(d_nr).name
                )
            })
    } else {
        None
    };
    let reg_name = if data.is_type_var_placeholder(d_nr) {
        // The generic type-var marker (`<T>`) is an INTERNAL compile-time construct.
        // Register its runtime type under a name a user type can never share, so
        // nothing it derives (`vector<T>`) collides in the name-keyed type table with
        // a user type of the same name (`enum T`) — which made the user's `vector<T>`
        // reuse the marker's size-0 entry and divide by zero.  The DEF name stays `T`
        // for stdlib `<T>` resolution; only the runtime type name is mangled.
        format!("__typevar_{}", data.def(d_nr).name)
    } else if let Some(name) = synth_variant_name {
        name
    } else if database.has_type(&data.def(d_nr).name) {
        // The source qualifier separates two LIBRARIES that define the same name.  It
        // cannot separate two definitions in ONE source, so a third same-named
        // structure aborted the compiler — `enum A { Nil, … } enum B { Nil, … }
        // enum C { Nil, … }` in one file is enough, and the abort read as an internal
        // error on legal code.  A variant's parent enum is itself a registered type,
        // so qualifying with the parent's DB name is unique by construction; it is the
        // same escape the synthetic `__nullable<S>` variants take above.  Reached only
        // once the source-qualified name is ALSO taken, so every program that compiles
        // today keeps the name it has.
        let qualified = data.qualified_type_name(d_nr);
        if database.has_type(&qualified) {
            variant_parent_qualified_name(data, database, d_nr).unwrap_or(qualified)
        } else {
            qualified
        }
    } else {
        data.def(d_nr).name.clone()
    };
    // `LOFT_TRACE_SCHEMA` — see `database::types::schema_trace`.  Logging the DEF
    // behind each registration is what makes a duplicate attributable: the abort
    // names only the colliding type, while the fault is one def being filled
    // twice (a rolled-back parse re-creating it), which shows up here as the same
    // `d_nr` registering a bare name and then a `src0::`-qualified one (#618).
    if std::env::var_os("LOFT_TRACE_SCHEMA").is_some() {
        eprintln!(
            "[schema] fill d_nr={d_nr} src={} name={:?} -> reg={reg_name:?}",
            data.def(d_nr).source,
            data.def(d_nr).name,
        );
    }
    let s_type = database.structure(&reg_name, enum_value);
    data.definitions[d_nr as usize].known_type = s_type;
    if data.def_type(d_nr) == DefType::EnumValue {
        let e_tp = data.def(d_nr).parent;
        let enum_tp = data.def(e_tp).known_type;
        database.enum_value(enum_tp, &data.def(d_nr).name, data.def(d_nr).known_type);
    }
    for a_nr in 0..data.attributes(d_nr) {
        // Computed fields are not stored — skip them in the database layout.
        if data.def(d_nr).attributes[a_nr].constant {
            continue;
        }
        let a_type = data.attr_type(d_nr, a_nr);
        // @PLN25 slice (b): an `Optional(τ)` field lays out exactly like `τ` (same sentinel
        // storage) — peel the marker here so the whole DB-layout path (the `db_type` match +
        // `size`) is transparent to it. Nullability is read separately via `attr_nullable`.
        let a_type = a_type.base().clone();
        let t_nr = data.type_elm(&a_type);
        let nullable = data.attr_nullable(d_nr, a_nr);
        if t_nr < u32::MAX {
            let tp = match a_type {
                Type::Vector(c_type, _) => {
                    let c_nr = data.type_elm(&c_type);
                    // unresolved vector content — parser already emitted
                    // a diagnostic (constant-shadow, undefined type, etc.).
                    // Skip this attribute rather than panicking so the user
                    // sees the proper error instead of an interpreter crash.
                    if c_nr == u32::MAX {
                        continue;
                    }
                    // route through the shared resolver so struct fields, locals,
                    // parameters, return types and literals all derive the element
                    // id the same way (narrow leaf, nested vector, plain
                    // `known_type`).  `None` = the leaf has no id yet, which is
                    // the one case this site can fix itself: fill it, then retry.
                    let c_tp = if let Some(elem) = data.vector_element_type(&c_type, database) {
                        elem
                    } else {
                        fill_database(data, database, c_nr);
                        data.vector_element_type(&c_type, database)
                            .unwrap_or(data.def(c_nr).known_type)
                    };
                    let tp = database.vector(c_tp);
                    data.check_vector(c_nr, tp, &data.def(d_nr).position.clone());
                    tp
                }
                Type::Integer(IntegerSpec {
                    min: minimum,
                    not_null,
                    forced_size: spec_forced,
                    ..
                }) => {
                    let field_nullable = nullable && !not_null;
                    // Post-2c: if the field's alias has a forced size(N)
                    // annotation, prefer it over the limit()-based heuristic.
                    // The alias def_nr was captured in parse_field because
                    // Type::Integer collapses alias names.
                    let alias = data.def(d_nr).attributes[a_nr].alias_d_nr;
                    // @PLN114 — fall back to the width the TYPE carries when the
                    // attribute has no alias to consult.  `parse_field` captures
                    // `alias_d_nr` for a declared struct field, but the synthetic
                    // `__tuple<…>` struct's attributes are built by `tuple_def` from
                    // element Types alone, so `forced_size(alias)` finds nothing and
                    // the range heuristic silently widens: `u8` became a 2-byte
                    // `short` and `u16` an 8-byte `integer`, which is why a tuple
                    // packed to 16 bytes where the identical record packs to 3.
                    // `IntegerSpec.forced_size` is already stamped by `parse_type`
                    // (definitions.rs:1869), so the fact is present — it just was
                    // not being read on this path.
                    let s = data
                        .forced_size(alias)
                        .or_else(|| spec_forced.map(std::num::NonZeroU8::get))
                        .unwrap_or_else(|| a_type.size(field_nullable));
                    if s == 1 {
                        database.byte(minimum, field_nullable)
                    } else if s == 2 {
                        // The schema Part MUST match the op the codegen chose via the ONE
                        // width→op home (`NarrowIntKind::of(2, nullable, narrow_vec=false)`):
                        // a NULLABLE 2-byte field is `Short` (the `+1` sentinel encoding), a
                        // NON-null one is `ShortFull` (direct, written via `OpSetShortRaw`).
                        // The schema READ (`ShowDb`/`to_json`/store round-trip) uses this Part,
                        // so a non-null field MUST be `Parts::ShortRaw` (direct decode) — using
                        // `Parts::Short` here made the read apply the `+1` shift the direct
                        // write never did, so a non-null `u16` field read back off-by-one
                        // (`7 → 6`) / as `i32::MIN` at the boundary, while field access (which
                        // already uses `OpGetShortFull`) was correct.  Pre-existing for
                        // `u16 not null`; F2 exposed it for plain `u16` (now non-null).
                        if field_nullable {
                            database.short(minimum, field_nullable)
                        } else {
                            database.short_raw(minimum, field_nullable)
                        }
                    } else if s == 4 {
                        database.int(minimum, field_nullable)
                    } else {
                        database.name("integer")
                    }
                }
                Type::Hash(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    let kd = key_bearing_def(data, c_nr);
                    // @PLN25 E2 — for a synth `__nullable<S>` element the keys live
                    // in the `Some` variant, which is built up-front by the
                    // eager-variant pass in `fill_all` (so `database.hash` resolves
                    // the key fields through it AND the hash's tid lands after
                    // `Some` for native codegen).  Safety net: if a hash is reached
                    // before that pass has run, build `Some` now so key resolution
                    // still succeeds (idempotent — a no-op once the pass has run).
                    if data.def(kd).known_type == u16::MAX {
                        fill_database(data, database, kd);
                    }
                    set_mutable(data, kd, &key_fields);
                    database.hash(c_tp, &key_fields)
                }
                Type::Index(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    // @PLN25 E2 — for a synth `__nullable<S>` element the key fields live in the
                    // `Some` payload, so resolve the key-bearing def (mirror the hash arm) before
                    // marking them immutable; `c_nr` (the enum) has no direct key attribute.
                    let kd = key_bearing_def(data, c_nr);
                    if data.def(kd).known_type == u16::MAX {
                        fill_database(data, database, kd);
                    }
                    set_mutable_directed(data, kd, &key_fields);
                    database.index(c_tp, &key_fields)
                }
                Type::Sorted(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    let kd = key_bearing_def(data, c_nr);
                    if data.def(kd).known_type == u16::MAX {
                        fill_database(data, database, kd);
                    }
                    set_mutable_directed(data, kd, &key_fields);
                    database.sorted(c_tp, &key_fields)
                }
                Type::Radix(c_nr, key_fields, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    set_mutable(data, c_nr, &key_fields);
                    database.spatial(c_tp, &key_fields)
                }
                Type::Trie(c_nr, key, _) => {
                    let mut c_tp = data.def(c_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, c_nr);
                        c_tp = data.def(c_nr).known_type;
                    }
                    set_mutable(data, c_nr, std::slice::from_ref(&key));
                    database.trie(c_tp, &key)
                }
                Type::Enum(t, _, _) if data.def(t).name == "enumerate" => database.byte(0, false),
                Type::Function(_, _, _) => {
                    // P213: when a capturing-lambda assignment has been
                    // seen at this attribute (its d_nr recorded on
                    // `assigned_lambda_d_nr` during first-pass parsing
                    // of `set_field_check`), split into TWO database
                    // fields:
                    //   `<attr>`              : 4B int, lambda d_nr
                    //   `<attr>__closure_rec` : `Parts::ChildRec(closure_kt)`,
                    //                           4B u32 rec-id of the
                    //                           co-located closure record
                    // For attributes that never received a capturing
                    // assignment (non-capturing fn-ref struct fields,
                    // tuple elements of fn-ref type, default-init only)
                    // stay with the legacy single-field 4B int layout —
                    // closure_rec field would be wasted space and
                    // breaks layouts of containers (tuples) that pre-
                    // computed positions assuming 4B per fn-ref slot.
                    let attr_name = data.def(d_nr).attributes[a_nr].name.clone();
                    let lambda_d = data.def(d_nr).attributes[a_nr].assigned_lambda_d_nr;
                    let closure_rec_d = if lambda_d == u32::MAX {
                        u32::MAX
                    } else {
                        data.def(lambda_d).closure_record
                    };
                    if closure_rec_d == u32::MAX {
                        // Legacy 4B int layout (non-capturing /
                        // tuple-element / default-init).
                        let int_tp = database.int(0, false);
                        database.field(s_type, &attr_name, int_tp);
                    } else {
                        let mut c_tp = data.def(closure_rec_d).known_type;
                        if c_tp == u16::MAX {
                            fill_database(data, database, closure_rec_d);
                            c_tp = data.def(closure_rec_d).known_type;
                        }
                        let dnr_tp = database.int(0, false);
                        let crec_tp = database.child_rec(c_tp);
                        database.field(s_type, &attr_name, dnr_tp);
                        database.field(s_type, &format!("{attr_name}__closure_rec"), crec_tp);
                    }
                    continue;
                }
                Type::Tuple(_) => {
                    // Plan-06 phase 4d: tuple struct fields inline the
                    // synthetic `__tuple<…>` struct's bytes.  The
                    // synthetic struct is registered eagerly by
                    // `parse_type_full`, but its database-side layout
                    // is built by `fill_database` on the synthetic
                    // def itself — recurse first so its `known_type`
                    // is non-`u16::MAX` when we register the host
                    // struct's tuple field below.  Mirrors the
                    // vector / sorted / hash / index recursion above.
                    let mut c_tp = data.def(t_nr).known_type;
                    if c_tp == u16::MAX {
                        fill_database(data, database, t_nr);
                        c_tp = data.def(t_nr).known_type;
                    }
                    c_tp
                }
                Type::Reference(_, ref deps) if !deps.is_empty() => {
                    // Plan-22 phase 02b (2026-05-12): auto-Reference
                    // encoding for mutated captures.  When the
                    // attribute's dep list is non-empty, the field
                    // holds a 12-byte `DbRef` pointing at the source
                    // record (shared storage) instead of inline
                    // bytes (deep-copy).  The dep list is the marker
                    // — phase 02c is the only producer that sets it
                    // for closure-record attributes; today's user
                    // code path always has empty deps so the
                    // legacy inline-bytes path stays active for
                    // every existing struct field.
                    //
                    // #682: which of the two markers decides whether the
                    // record ADOPTS the captured store (cascade-freed with
                    // the record) or merely BORROWS it (freed by its real
                    // owner).  Same 12 bytes either way — `generation` picks
                    // the same pair for `--native`.
                    if deps.is_borrowed_share() {
                        database.dbref_borrow()
                    } else {
                        database.dbref()
                    }
                }
                _ => {
                    // A struct/enum-reference field stored INLINE (`inner: Cell`,
                    // empty deps) — its bytes live inside the host record, so the
                    // host layout needs the content type's size now.  The host can
                    // be declared BEFORE the content (a forward or cross-package
                    // reference), in which case the content's `known_type` is still
                    // u16::MAX here.  Lay it out first — mirroring the vector /
                    // tuple / keyed-collection recursion above — otherwise the
                    // field's content id stays u16::MAX, `finish_type` cannot
                    // position it (the field lands at offset u16::MAX, never
                    // repaired on pass 2 because `finish_type` skips an
                    // already-sized type), and codegen reads the bogus offset and
                    // corrupts the free path (@P373: a SIGSEGV at scope exit
                    // AFTER the correct value prints).  Primitive fields already
                    // carry a real `known_type`, so the guard never recurses for
                    // them; a genuinely-undefined `Unknown(0)` stub short-circuits
                    // in `fill_database` (already diagnosed elsewhere).
                    let mut kt = data.def(t_nr).known_type;
                    if kt == u16::MAX {
                        fill_database(data, database, t_nr);
                        kt = data.def(t_nr).known_type;
                    }
                    kt
                }
            };
            database.field(s_type, &data.attr_name(d_nr, a_nr), tp);
            // @PLN127 arc D — the ONE parse-time site that knows. `a_type` was
            // peeled above (an `Optional(τ)` lays out exactly like `τ`), so
            // without depositing it here the fact is gone by the time anything
            // can be asked about it.
            if nullable {
                database.set_field_nullable(s_type, &data.attr_name(d_nr, a_nr), true);
            }
        }
    }
    // Propagate Data-side LinkedFieldGroups (currently: tuple element
    // groups registered by `tuple_def`) to the Database-side Type so
    // `Stores::finish_type` can place them atomically via
    // `calculate_positions_with_groups`.  Index bookkeeping groups
    // are added directly on the Database side by `Stores::index`, so
    // they don't need this copy.
    let groups = data.def(d_nr).field_groups.clone();
    if !groups.is_empty() {
        database.types[s_type as usize].field_groups.extend(groups);
    }
}

/// @PLN25 E2 — the key-bearing struct for a keyed collection.  When the element
/// was rewritten to the synthetic `__nullable<S>` enum (E2), its key field(s) live
/// inside the `Some` variant, not at the enum's top level, so key-field name lookups
/// must resolve against `Some` (whose payload offsets match the Some-wrapped records
/// the collection shares with its sibling vector).  A non-synthetic content def is
/// returned unchanged — gate-inert.
pub(crate) fn key_bearing_def(data: &Data, c_nr: u32) -> u32 {
    if data.def_type(c_nr) == DefType::Enum && data.def(c_nr).name.starts_with("__nullable<") {
        let some = data.variant_of(c_nr, "Some");
        if some != u32::MAX {
            // Single-payload: the key fields live inside the `Some` variant's inline
            // `payload` field (a dense `S`), so the key-bearing def is the payload's
            // struct, not the `Some` variant (whose direct fields are {enum, payload}).
            let payload_attr = data.attr(some, "payload");
            if payload_attr != usize::MAX
                && let Type::Reference(struct_d, _) =
                    data.def(some).attributes()[payload_attr].typedef
            {
                return struct_d;
            }
        }
    }
    c_nr
}

fn set_mutable(data: &mut Data, on_d: u32, fields: &[String]) {
    for f in fields {
        let a_nr = data.attr(on_d, f);
        data.definitions[on_d as usize].attributes[a_nr].mutable = false;
    }
}

fn set_mutable_directed(data: &mut Data, on_d: u32, fields: &[(String, bool)]) {
    for f in fields {
        let a_nr = data.attr(on_d, &f.0);
        data.definitions[on_d as usize].attributes[a_nr].mutable = false;
    }
}
