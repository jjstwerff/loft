// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN153 phase 0 — the `τ??` census: an OBSERVER over every type a compiled program carries.
//!
//! `types.md (N-Idem)` says `τ?? ≡ τ?`, and [`crate::data::Type::optional`] is the idempotent
//! former that makes it so — but it is not the only way an `Optional` is built.  Thirteen sites
//! construct `Type::Optional(Box::new(…))` directly, and one of them re-wraps a field's `?`
//! around a type that was resolved AFTER the `?` was peeled, so a `τ??` can only come from a
//! route the former never sees.  A rule with one home and a second spelling beside it is the
//! shape every walk in QUALITY.md finds, and this is the instrument that says whether the
//! second spelling has ever produced the thing the rule forbids.
//!
//! Gated on `LOFT_NULL_CENSUS` because `[profile.dev.package.loft]` compiles a `debug_assert`
//! OUT of this library — an assertion here would be absent from every build that runs the
//! corpus.  Off, it costs one env read.  On, it prints one line per program to stderr, with a
//! `where` line for every nested optional it finds, so a corpus sweep can grep the count and
//! the location alike.
//!
//! Non-vacuity is proved in the unit tests below rather than by a runtime injection: a
//! hand-built `Optional(Optional(Integer))` makes [`nested_optionals_in`] read 1, so a corpus
//! that reads 0 read it with an instrument that can say otherwise.

use crate::data::{Data, Type};

/// How many `Optional` nodes in `tp` sit DIRECTLY under another `Optional` — the count of
/// `τ??` shapes, at any depth (a `vector<integer??>` counts once, for the element).
pub fn nested_optionals_in(tp: &Type) -> usize {
    fn walk(tp: &Type, under_optional: bool, out: &mut usize) {
        let here = matches!(tp, Type::Optional(_));
        if here && under_optional {
            *out += 1;
        }
        tp.for_each_child(&mut |c| walk(c, here, out));
    }
    let mut n = 0;
    walk(tp, false, &mut n);
    n
}

/// Every type the program carries, with a name for where it lives: a definition's declared
/// return, each of its attributes, and each of its function's variables.
fn each_type(data: &Data, mut f: impl FnMut(String, &Type)) {
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        let name = def.name().to_string();
        f(format!("{name} -> return"), def.returned());
        for (i, a) in def.attributes().iter().enumerate() {
            f(format!("{name}.{} (attr {i})", a.name), &a.typedef);
        }
        let vars = &def.variables;
        for v in 0..vars.count() {
            f(format!("{name}::{} (var {v})", vars.name(v)), vars.tp(v));
        }
    }
}

/// The census over one compiled program.  Answers `(types scanned, nested optionals, where)`;
/// [`report`] is the env-gated printer over it.
pub fn census(data: &Data) -> (usize, usize, Vec<String>) {
    let mut scanned = 0usize;
    let mut nested = 0usize;
    let mut where_ = Vec::new();
    each_type(data, |place, tp| {
        scanned += 1;
        let n = nested_optionals_in(tp);
        if n > 0 {
            nested += n;
            where_.push(format!("{place}: {tp}"));
        }
    });
    (scanned, nested, where_)
}

/// Print the census to stderr when `LOFT_NULL_CENSUS` is set; a no-op otherwise.
pub fn report(data: &Data) {
    if std::env::var_os("LOFT_NULL_CENSUS").is_none() {
        return;
    }
    let (scanned, nested, where_) = census(data);
    eprintln!("null-census: types-scanned={scanned} nested-optional={nested}");
    for w in where_ {
        eprintln!("null-census: where {w}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Deps;

    /// A scalar leaf with no spec to build — the nesting is what is under test, not the leaf.
    fn int() -> Type {
        Type::Boolean
    }

    #[test]
    fn a_plain_and_a_single_optional_read_zero() {
        assert_eq!(nested_optionals_in(&int()), 0);
        assert_eq!(nested_optionals_in(&Type::optional(int())), 0);
        assert_eq!(
            nested_optionals_in(&Type::Vector(Box::new(Type::optional(int())), Deps::none())),
            0
        );
    }

    /// The non-vacuity control: the instrument can say 1, so a corpus reading 0 said something.
    #[test]
    fn a_hand_built_nested_optional_reads_one() {
        let nested = Type::Optional(Box::new(Type::Optional(Box::new(int()))));
        assert_eq!(nested_optionals_in(&nested), 1);
        // …and at depth, through a container, it is still found and still counts once.
        let deep = Type::Vector(Box::new(nested), Deps::none());
        assert_eq!(nested_optionals_in(&deep), 1);
        // A triple counts two nestings, so the number is a count and not a flag.
        let triple = Type::Optional(Box::new(Type::Optional(Box::new(Type::Optional(
            Box::new(int()),
        )))));
        assert_eq!(nested_optionals_in(&triple), 2);
    }

    /// The former itself never builds one — `(N-Idem)`'s home, measured rather than assumed.
    #[test]
    fn the_former_is_idempotent() {
        let once = Type::optional(int());
        let twice = Type::optional(once.clone());
        assert_eq!(once, twice);
        assert_eq!(nested_optionals_in(&twice), 0);
    }
}
