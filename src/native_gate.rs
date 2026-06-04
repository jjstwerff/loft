// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 Arc N / N2 — the **native-compilability gate**.
//!
//! In the native-library execution model (C71) a stable library compiles to
//! native code while the user script interprets, and calls dispatch across the
//! shared store ABI.  Not every function can be `--native`-compiled, so the
//! compiler partitions a library's functions into the **maximal native subgraph**
//! (compiled, dispatched via `OpStaticCall`) and the rest (run by the
//! interpreter).  This module computes that partition.
//!
//! **The denylist is small.**  Investigating the native backend (`generation/`)
//! found it emits *everything* — structs, enums, vectors, **generics, closures** —
//! except the concurrency constructs `parallel{}` / `par_for` / `yield`, for which
//! `emit.rs` writes a non-code comment.  (`NATIVE_SKIP` / `SCRIPTS_NATIVE_SKIP` in
//! `tests/native.rs` are both empty — the whole corpus compiles.)  So the
//! generics/closures "research problem" the plan feared was a phantom; the gate is
//! simply "uses no concurrency construct, transitively."
//!
//! **The gate is transitive and conservative** (Goal F — when unsure, interpret):
//! a function is native iff its body *and every function it `Call`s* are native.
//! This keeps the native subgraph **closed** — a native function only calls native
//! functions — so the runtime boundary is only ever *interpret → native*
//! (`OpStaticCall`, the supported direction); a native function never needs the
//! hard native → interpreter upcall.  A `CallRef` (runtime fn-ref) is excluded:
//! its dynamic callee can't be proven native.

use crate::data::{Data, DefType, Type, Value};
use std::collections::HashSet;

/// @PLAN54 Arc N / N2 — the set of function `def_nr`s that can be `--native`-
/// compiled (the maximal native subgraph).  Everything not in the set runs
/// interpreted.  See the module docs for the gate's denylist + transitivity.
#[must_use]
pub fn native_compilable(data: &Data) -> HashSet<u32> {
    (0..data.definitions())
        .filter(|&d| {
            matches!(data.def(d).def_type(), DefType::Function)
                && fn_native_compilable(data, d, &mut HashSet::new())
        })
        .collect()
}

/// @PLAN54 Arc N / N2 — the **scalar-dispatchable** subset: native-compilable
/// functions whose parameters and return type are all scalars
/// (`integer`/`boolean`/`float`/`single`/`character`).  These are the **first
/// dispatch slice**: no store reference crosses the interpreter↔native boundary,
/// so the auto-generated `#native` export wrapper is trivial and the
/// `&UnsafeCell<Stores>` ↔ `LoftStore` bridge is not needed (any internal store
/// use is contained — a scalar-return function cannot leak a store ref out).  The
/// store-touching functions (Text / Vector / Reference signatures) come in the
/// next slice with the bridge.
#[must_use]
pub fn scalar_dispatchable(data: &Data) -> HashSet<u32> {
    native_compilable(data)
        .into_iter()
        .filter(|&d| {
            let def = data.def(d);
            is_scalar_type(def.returned())
                && def
                    .attributes()
                    .iter()
                    // skip synthetic / hidden params (`__closure`, work buffers, …)
                    .filter(|a| !a.name.starts_with("__"))
                    .all(|a| is_scalar_type(&a.typedef))
        })
        .collect()
}

/// A type that passes the interpreter↔native boundary by value, with no store
/// reference — the trivially-marshallable set.  (Plain enums, `Text`, vectors and
/// references are deliberately excluded from this first slice.)
fn is_scalar_type(t: &Type) -> bool {
    matches!(
        t,
        Type::Integer(_) | Type::Boolean | Type::Float | Type::Single | Type::Character
    )
}

/// @PLAN54 Arc N / N2 — the **shared-store-dispatchable** subset: native-compilable
/// functions that dispatch over the **shared `*mut Stores` bridge**
/// (`generate_shared_cdylib_lib_rs`): the caller's store is shared by pointer (no
/// `LoftStore` handle, no marshalling), so a non-scalar value can cross the
/// boundary.
///
/// **Parameters** may be any bridge type — scalars **plus** the `DbRef`-backed
/// aggregates (`vector`/`reference`/data-`enum`/sorted/index/hash/spacial), passed
/// as the raw stack `DbRef`.  **Returns** may be scalar, `void`, or a `vector`.  A
/// `vector` return triggers `--native`'s **hidden destination-parameter** convention
/// (`ref_return` appends a trailing `DbRef` work-vector marked `Attribute::hidden`
/// that the caller pre-allocates with `stores.null_named` + `OpDatabase(<type_id>)`);
/// the bridge wrapper allocates that destination itself, so the script-side
/// `#native` forward-decl still models only the public params.
///
/// A struct `reference` return is supported (no hidden dest — the body allocates
/// the record fresh).  `Text` params (`&str`) and `Text` returns (via the
/// `text_return` `&mut String` work buffer the bridge owns) are supported.  Not
/// yet handled (a function is excluded if any holds): a data-`enum` return; a
/// `__closure` (or other non-text-work `__`-prefixed) synthetic param.
#[must_use]
pub fn shared_store_dispatchable(data: &Data) -> HashSet<u32> {
    native_compilable(data)
        .into_iter()
        .filter(|&d| {
            let def = data.def(d);
            let ret = def.returned();
            // A vector return uses `--native`'s hidden destination param (the bridge
            // allocates it); a struct `reference` return does NOT (the body
            // allocates the record fresh and returns its `DbRef` —
            // `n_make_point(cell, a, b) -> DbRef`), so it needs no hidden dest.  A
            // `text` return uses `text_return`'s `&mut String` work buffer (the
            // bridge owns a local `String` and copies the result into scratch).
            let ret_text = matches!(ret, Type::Text(_));
            let ret_ok = matches!(ret, Type::Void | Type::Null)
                || is_scalar_type(ret)
                || matches!(ret, Type::Vector(_, _) | Type::Reference(_, _))
                || ret_text;
            ret_ok
                && def.attributes().iter().all(|a| {
                    if a.hidden {
                        // The only hidden destination the bridge allocates is a vector.
                        matches!(a.typedef, Type::Vector(_, _))
                    } else if crate::native_lib::is_text_work_buffer(&a.typedef) {
                        // text_return `&mut String` work buffer — valid iff the
                        // function actually returns text.
                        ret_text
                    } else if a.name.starts_with("__") {
                        false // closures — not handled
                    } else {
                        is_bridge_type(&a.typedef)
                    }
                })
        })
        .collect()
}

/// A type passable through the shared-store `LibArg` bridge **as a parameter**: a
/// scalar (by value), a `text` (as `&str` — ptr+len borrowed from the shared
/// store), or a `DbRef`-backed aggregate (`vector`/`reference`/data-`enum`/sorted/
/// index/hash/spacial), the latter shared by pointer through the caller's store.
/// (A `text` *return* is gated separately — it needs the `text_return` work
/// buffer, not just a slot.)
fn is_bridge_type(t: &Type) -> bool {
    is_scalar_type(t)
        || matches!(
            t,
            Type::Text(_)
                | Type::Vector(_, _)
                | Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::Sorted(_, _, _)
                | Type::Index(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Spacial(_, _, _)
        )
}

/// Transitive native-compilability of function `d_nr`: its body uses no
/// un-native-able construct AND every function it `Call`s is itself native.
/// `visited` breaks recursion cycles optimistically (a recursive self-call is
/// treated as native — the function's own body is checked on its first visit),
/// mirroring `scopes::walk_par_safe`.
fn fn_native_compilable(data: &Data, d_nr: u32, visited: &mut HashSet<u32>) -> bool {
    if d_nr == u32::MAX || d_nr >= data.definitions() {
        return false; // unknown / out-of-range callee → conservative
    }
    if !visited.insert(d_nr) {
        return true; // cycle — already being checked higher up the stack
    }
    let def = data.def(d_nr);
    if !matches!(def.def_type(), DefType::Function) {
        return false;
    }
    walk(def.code(), data, visited)
}

/// Exhaustive walk of a `Value` tree — **no `_` arm by design**: a future IR
/// variant must be classified here, so an un-native-able construct can never
/// silently slip through as "native" and produce broken Rust (Goal F:
/// conservative-silent, never a compile error in the user's face).  Returns
/// `false` the moment an un-native-able construct (or a non-native callee) is hit.
fn walk(v: &Value, data: &Data, visited: &mut HashSet<u32>) -> bool {
    match v {
        // The native backend cannot emit these (concurrency / coroutine) — see
        // `generation/emit.rs` (Parallel/ParFor emit a non-code comment).
        Value::Parallel(_) | Value::ParFor(_) | Value::Yield(_) => false,
        // Dynamic dispatch: the runtime callee is unknown → can't prove native.
        Value::CallRef(_, _) => false,
        // A static call: the callee must itself be native, and so must the args.
        Value::Call(callee, args) => {
            fn_native_compilable(data, *callee, visited)
                && args.iter().all(|a| walk(a, data, visited))
        }
        // Structural nodes carrying children — recurse into all of them.
        Value::Span(b) => walk(&b.1, data, visited),
        Value::Block(b) | Value::Loop(b) => b.operators.iter().all(|x| walk(x, data, visited)),
        Value::Insert(vs) | Value::Tuple(vs) => vs.iter().all(|x| walk(x, data, visited)),
        Value::Set(_, x)
        | Value::Return(x)
        | Value::BreakWith(_, x)
        | Value::Drop(x)
        | Value::TuplePut(_, _, x) => walk(x, data, visited),
        Value::If(c, t, e) => {
            walk(c, data, visited) && walk(t, data, visited) && walk(e, data, visited)
        }
        Value::Iter(_, create, next, init) => {
            walk(create, data, visited) && walk(next, data, visited) && walk(init, data, visited)
        }
        // Leaves — literals, var reads, fn-ref construction, keys, raw-expr.
        Value::Null
        | Value::Line(_)
        | Value::Int(_)
        | Value::Enum(_, _)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Long(_)
        | Value::Single(_)
        | Value::Text(_)
        | Value::Var(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::TupleGet(_, _)
        | Value::FnRef(_, _, _)
        | Value::FnRefDnr(_)
        | Value::RawExpr(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::boxed::Box as b; // terse `b::new(..)` for building test IR

    #[test]
    fn walk_classifies_leaves_and_denylist() {
        let data = Data::new();
        let mut seen = HashSet::new();
        // Leaves are native-OK.
        assert!(walk(&Value::Int(1), &data, &mut seen));
        assert!(walk(&Value::Text("x".into()), &data, &mut seen));
        // The concurrency / coroutine constructs are not.
        assert!(!walk(&Value::Parallel(vec![]), &data, &mut seen));
        assert!(!walk(&Value::Yield(b::new(Value::Null)), &data, &mut seen));
        // Dynamic dispatch is conservatively excluded.
        assert!(!walk(&Value::CallRef(0, vec![]), &data, &mut seen));
    }

    #[test]
    fn walk_finds_nested_denylist_construct() {
        let data = Data::new();
        let mut seen = HashSet::new();
        // A `parallel` buried in a branch must still flip the verdict — the walk
        // is exhaustive, so nesting cannot hide it.
        let nested = Value::If(
            b::new(Value::Boolean(true)),
            b::new(Value::Parallel(vec![Value::Int(1)])),
            b::new(Value::Null),
        );
        assert!(!walk(&nested, &data, &mut seen));
        // The same shape without the parallel arm is native.
        let clean = Value::If(
            b::new(Value::Boolean(true)),
            b::new(Value::Int(1)),
            b::new(Value::Null),
        );
        assert!(walk(&clean, &data, &mut HashSet::new()));
    }

    /// On the real stdlib the native backend compiles essentially everything, so
    /// the gate should classify the bulk of stdlib functions as native — and the
    /// total is non-trivial.  (Prints the coverage for visibility.)
    #[test]
    fn stdlib_is_mostly_native_compilable() {
        let mut p = crate::parser::Parser::new();
        if p.parse_dir("default", true, false).is_err() {
            eprintln!("skip: default/ stdlib not parseable here");
            return;
        }
        let total_fns = (0..p.data.definitions())
            .filter(|&d| matches!(p.data.def(d).def_type(), DefType::Function))
            .count();
        let native = native_compilable(&p.data);
        eprintln!(
            "native gate: {}/{} stdlib functions native-compilable",
            native.len(),
            total_fns
        );
        assert!(total_fns > 0, "stdlib should have functions");
        assert!(
            native.len() * 2 > total_fns,
            "most stdlib functions should be native-compilable ({}/{})",
            native.len(),
            total_fns
        );
    }

    /// The scalar-dispatchable subset (the first dispatch slice) is a non-empty
    /// subset of the native set — the stdlib has many scalar-signature functions.
    #[test]
    fn scalar_dispatchable_is_a_nonempty_subset() {
        let mut p = crate::parser::Parser::new();
        if p.parse_dir("default", true, false).is_err() {
            eprintln!("skip: default/ stdlib not parseable here");
            return;
        }
        let native = native_compilable(&p.data);
        let scalar = scalar_dispatchable(&p.data);
        eprintln!(
            "native gate: {}/{} native functions are scalar-dispatchable (first slice)",
            scalar.len(),
            native.len()
        );
        assert!(
            scalar.is_subset(&native),
            "scalar-dispatchable must be a subset of native-compilable"
        );
        assert!(
            !scalar.is_empty(),
            "stdlib should have scalar-only functions"
        );
        assert!(
            scalar.len() < native.len(),
            "not every native function is scalar-only (text/vector/ref signatures exist)"
        );
    }
}
