// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN97 Phase B — the golden layout-conformance test (the instrument).
//
// Pins the EXACT store layout of a corpus of loft structures via the reusable
// `Stores::layout_dump` / `Stores::layout_algo_hash` (src/database/types.rs — the
// same API phases D/F consume): for each type its record `size` and its `Parts`
// descriptor rendered structurally — struct field byte positions, narrow-int
// encodings (#399), and collection element strides (the nested-vector stride
// #477 changed). The store is ONE format (bit-identical in memory and on disk),
// and this layout is a deterministic function of the type table shared by BOTH
// backends — so pinning it here makes any layout change a red diff at commit time
// instead of silently-invalidated data (the gap @PLN97 exists to close).
//
// A layout change is not forbidden — it must be DELIBERATE: the golden diff shows
// exactly what moved, and `LAYOUT_ALGO_HASH` must be re-blessed in the same change
// (that constant is the Phase-D handoff hash).
//
// Regenerate after an intentional layout change:
//   LOFT_BLESS_LAYOUT=1 cargo test --release --test layout_golden layout_golden
// then hand-verify the diff and update LAYOUT_ALGO_HASH to the printed value.

mod common;
extern crate loft;

use common::cached_default;
use loft::data::Data;
use loft::database::{Parts, Stores};
use loft::parser::Parser;
use std::path::PathBuf;

/// The corpus — one structure per representative layout cell. Closure (fn-ref
/// field → DbRef/ChildRec), Radix (1.1+), and Sorted/Short (a local / nullable
/// narrow) stay uncovered — the coverage audit classifies them (Phase B4).
const CORPUS: &str = r#"
struct Scalars { b: boolean, c: character, s: single, f: float, i: integer, t: text }
struct Narrow { a: i32, b: u8, c: u16 }
struct Wide { s: i16 }
struct NotNull { i: integer not null, t: text not null }
struct Vec1 { v: vector<integer> }
struct VecNest { vv: vector<vector<integer>> }
struct VecText { v: vector<text> }
struct Item { ik: integer }
struct Bag { items: hash<Item[ik]> }
struct SortedBag { items: sorted<Item[ik]> }
struct IndexBag { items: index<Item[ik]> }
struct RefHost { child: Scalars }
struct Tup { pair: (integer, text) }
enum Color { Red, Green, Blue }
enum Shape { Circle { radius: integer }, Rect { width: integer, height: integer } }
"#;

/// The corpus roots — the closure over their referenced types is what the golden
/// pins and the audit classifies.
const TYPES: &[&str] = &[
    "Scalars",
    "Narrow",
    "Wide",
    "NotNull",
    "Vec1",
    "VecNest",
    "VecText",
    "Item",
    "Bag",
    "SortedBag",
    "IndexBag",
    "RefHost",
    "Tup",
    "Color",
    "Shape",
];

/// A layout change flips this. Re-bless (with the golden) on an intentional change.
/// (@PLN97 F9 2026-07-17 — re-blessed after adding the `@endian` line to the layout dump.)
const LAYOUT_ALGO_HASH: u64 = 14_478_928_954_060_894_342;

/// @PLN102 arc-E flip-gate (Gate 1 step 3) — the `contract` version at which the
/// CURRENT layout was frozen. The store layout IS the persistence contract, so
/// POST-FLIP a change to `LAYOUT_ALGO_HASH` / the golden may land only alongside a
/// `CONTRACT_VERSION` bump (a declared, epoch-style break) — enforced git-side by
/// `scripts/check_contract_goldens.sh`. When you re-bless the layout at
/// `CONTRACT_VERSION > 0`, set this to the new `CONTRACT_VERSION` too. INERT while
/// `CONTRACT_VERSION == 0` (pre-freeze: the language is still settling, layout
/// changes are free). Invariant: `LAYOUT_CONTRACT <= CONTRACT_VERSION` always — a
/// layout cannot be frozen at a contract the runtime has not reached.
const LAYOUT_CONTRACT: u32 = 0;

/// The corpus roots resolved to known_types (loudly — a parse / syntax drift fails).
fn corpus_roots(data: &Data) -> Vec<u16> {
    TYPES
        .iter()
        .map(|name| {
            let kt = data.def(data.def_nr(name)).known_type;
            assert!(
                kt != u16::MAX,
                "corpus type `{name}` did not resolve to a known_type — parse failed or syntax drifted"
            );
            kt
        })
        .collect()
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/layout/corpus.txt")
}

#[test]
fn layout_golden() {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(CORPUS, "layout_corpus", false);

    let roots = corpus_roots(&p.data);
    let dump = p.database.layout_dump(&roots);
    let path = golden_path();

    if std::env::var("LOFT_BLESS_LAYOUT").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &dump).unwrap();
        eprintln!(
            "blessed {} ; LAYOUT_ALGO_HASH = {}",
            path.display(),
            p.database.layout_algo_hash(&roots)
        );
        eprintln!(
            "@PLN102 flip-gate: at CONTRACT_VERSION={}, a layout re-bless is a \
             persistence-contract change — post-freeze, bump CONTRACT_VERSION and set \
             LAYOUT_CONTRACT = CONTRACT_VERSION (inert at 0).",
            loft::manifest::CONTRACT_VERSION,
        );
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {} — regenerate with LOFT_BLESS_LAYOUT=1",
            path.display()
        )
    });
    assert_eq!(
        dump, expected,
        "\nSTORE LAYOUT CHANGED. If intentional: re-bless (LOFT_BLESS_LAYOUT=1), \
         hand-verify the diff, and update LAYOUT_ALGO_HASH. If not: a layout \
         regression (the #477 class) just got caught.\n"
    );
    assert_eq!(
        p.database.layout_algo_hash(&roots),
        LAYOUT_ALGO_HASH,
        "layout-algo hash drifted from the pinned constant"
    );
}

/// @PLN102 arc-E flip-gate (Gate 1 step 3) — the layout is frozen at `LAYOUT_CONTRACT`;
/// the running `CONTRACT_VERSION` can never be BELOW it (a layout cannot be frozen at a
/// contract the runtime has not reached). This pins the in-tree half of the coupling;
/// the "a layout change requires a contract bump" half is git-diff-shaped and lives in
/// `scripts/check_contract_goldens.sh` (also inert while `CONTRACT_VERSION == 0`). When
/// the two are equal the layout is frozen at the current contract — the normal state.
#[test]
fn layout_contract_pin_is_consistent() {
    // `black_box` so the check reads as the runtime comparison it becomes once the two
    // diverge (post-flip) — not a const-folded tautology clippy would flag while both
    // are 0 (inert). The invariant is real: a re-bless at CONTRACT_VERSION>0 must raise
    // LAYOUT_CONTRACT to match, so LAYOUT_CONTRACT can never exceed the running contract.
    let frozen_at = std::hint::black_box(LAYOUT_CONTRACT);
    let running = std::hint::black_box(loft::manifest::CONTRACT_VERSION);
    assert!(
        frozen_at <= running,
        "LAYOUT_CONTRACT ({frozen_at}) exceeds CONTRACT_VERSION ({running}) — a layout \
         cannot be frozen at a contract the runtime has not reached; re-bless the layout \
         (LOFT_BLESS_LAYOUT=1) and set LAYOUT_CONTRACT = CONTRACT_VERSION",
    );
}

// ── B4 — coverage self-audit ────────────────────────────────────────────────
//
// Guarantees "every structure" stays true as loft grows. Three teeth:
//  1. `coverage()` is EXHAUSTIVE over `Parts` — a NEW storage kind fails to
//     COMPILE here, forcing whoever adds it to declare its layout-test coverage.
//  2. Every kind the corpus produces must be classified `Covered` (a ratchet:
//     add a corpus entry that produces a `Gap` kind → this fails → promote it).
//  3. The corpus produces EXACTLY the `Covered` kinds (delete a corpus entry
//     that drops a covered kind → this fails).

/// The types a type's layout references — the closure the audit classifies.
/// (`Stores::layout_dump`/`layout_algo_hash` carry the same walk internally.)
fn referenced(db: &Stores, kt: u16, out: &mut Vec<u16>) {
    match &db.types[kt as usize].parts {
        Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
            out.extend(fields.iter().map(|f| f.content));
        }
        Parts::Vector(e) | Parts::Array(e) => out.push(*e),
        Parts::Sorted(e, _)
        | Parts::Ordered(e, _)
        | Parts::Hash(e, _)
        | Parts::Index(e, _, _)
        | Parts::Radix(e, _) => out.push(*e),
        Parts::ChildRec(c) => out.push(*c),
        // A plain variant (no data) keeps `known_type == u16::MAX`; only
        // data-carrying variants have an `EnumValue` type to reach.
        Parts::Enum(vs) => out.extend(vs.iter().map(|(t, _)| *t).filter(|t| *t != u16::MAX)),
        _ => {}
    }
}

fn reached_types(db: &Stores, data: &Data) -> std::collections::BTreeSet<u16> {
    let mut seen: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
    let mut stack: Vec<u16> = corpus_roots(data);
    while let Some(kt) = stack.pop() {
        if !seen.insert(kt) {
            continue;
        }
        let mut refs = Vec::new();
        referenced(db, kt, &mut refs);
        for r in refs {
            if !seen.contains(&r) {
                stack.push(r);
            }
        }
    }
    seen
}

#[derive(PartialEq, Eq, Debug)]
enum Cover {
    /// Exercised by the corpus — its layout is pinned by the golden.
    Covered,
    /// A real user-writable storage kind NOT yet in the corpus — add it.
    Gap(&'static str),
    /// Not a user-declared shape (codegen-only) — no corpus entry expected.
    Internal(&'static str),
}

/// EXHAUSTIVE over `Parts`. A new variant makes this a non-exhaustive match =
/// a compile error, so no storage kind can be added without a coverage verdict.
fn coverage(p: &Parts) -> (&'static str, Cover) {
    match p {
        Parts::Base => ("Base", Cover::Covered),
        Parts::Struct(_) => ("Struct", Cover::Covered),
        Parts::Byte(..) => ("Byte", Cover::Covered),
        Parts::ShortRaw(..) => ("ShortRaw", Cover::Covered),
        Parts::Int(..) => ("Int", Cover::Covered),
        Parts::Vector(_) => ("Vector", Cover::Covered),
        Parts::Hash(..) => ("Hash", Cover::Covered),
        Parts::Enum(_) => ("Enum", Cover::Covered),
        Parts::EnumValue(..) => ("EnumValue", Cover::Covered),
        Parts::Index(..) => ("Index", Cover::Covered),
        // `sorted<T[k]>` as a struct field is array-backed → Ordered.
        Parts::Ordered(..) => ("Ordered", Cover::Covered),
        // The vector-backed `sorted` (Parts::Sorted) needs a local `sorted<T[k]>=[]`,
        // not a struct field — not in the corpus yet.
        Parts::Sorted(..) => (
            "Sorted",
            Cover::Gap("vector-backed sorted — a local, not a field"),
        ),
        // The 2-byte SHIFTED narrow int (distinct from ShortRaw); a nullable
        // narrow field, not produced by i16/u16 (those are ShortRaw).
        Parts::Short(..) => (
            "Short",
            Cover::Gap("2-byte shifted narrow int — nullable narrow field"),
        ),
        Parts::Radix(..) => (
            "Radix",
            Cover::Gap("spatial<T[key]> — planned 1.1+, errors today"),
        ),
        Parts::Array(_) => (
            "Array",
            Cover::Internal("codegen-only reference collection"),
        ),
        Parts::DbRef => (
            "DbRef",
            Cover::Internal("12B stored DbRef — fn-ref closure half"),
        ),
        Parts::ChildRec(_) => (
            "ChildRec",
            Cover::Internal("closure-in-struct-field codegen"),
        ),
    }
}

/// The storage kinds the corpus is expected to produce — kept in lockstep with
/// the `Cover::Covered` arms above by the audit's exact-set assertion.
const COVERED_LABELS: &[&str] = &[
    "Base",
    "Struct",
    "Byte",
    "ShortRaw",
    "Int",
    "Vector",
    "Hash",
    "Ordered",
    "Index",
    "Enum",
    "EnumValue",
];

#[test]
fn layout_coverage_audit() {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(CORPUS, "layout_corpus", false);

    let mut produced: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for kt in reached_types(&p.database, &p.data) {
        let (label, cover) = coverage(&p.database.types[kt as usize].parts);
        assert_eq!(
            cover,
            Cover::Covered,
            "corpus now produces storage kind `{label}` (classified {cover:?}) — a gap just \
             closed: promote it to Cover::Covered and add it to COVERED_LABELS"
        );
        produced.insert(label);
    }

    let expected: std::collections::BTreeSet<&'static str> =
        COVERED_LABELS.iter().copied().collect();
    assert_eq!(
        produced, expected,
        "corpus storage-kind coverage drifted: the corpus must produce EXACTLY the \
         Cover::Covered kinds. Missing → a corpus structure was removed; extra → promote \
         the new kind in coverage() + COVERED_LABELS."
    );
}

// ── Phase D — the schema-description sidecar (self-describing store) ─────────

/// The in-memory layout identity built over the real corpus round-trips through
/// the `.dschema` sidecar text, carries the same pinned hash the golden asserts,
/// and an UNCHANGED store hands over raw (`Handoff::Identical`).
#[test]
fn schema_sidecar_identity_end_to_end() {
    use loft::schema_sidecar::{Handoff, LayoutIdentity, classify};

    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(CORPUS, "layout_corpus", false);
    let roots = corpus_roots(&p.data);

    let id = LayoutIdentity::of(&p.database, &roots);
    assert_eq!(
        id.layout_hash, LAYOUT_ALGO_HASH,
        "sidecar identity hash must match the golden's pinned layout hash"
    );
    assert_eq!(
        LayoutIdentity::from_sidecar(&id.to_sidecar()),
        Some(id.clone()),
        "sidecar text must round-trip"
    );
    assert_eq!(
        classify(&id, &id),
        Handoff::Identical,
        "an unchanged store hands over raw"
    );
}

/// `program_roots` (Phase F) returns exactly the user-defined struct/enum types
/// — the same set the corpus roots name — excluding stdlib and enum variants.
#[test]
fn program_roots_are_the_user_types() {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(CORPUS, "layout_corpus", false);

    let roots: std::collections::BTreeSet<u16> = loft::schema_sidecar::program_roots(&p.data)
        .into_iter()
        .collect();
    let expected: std::collections::BTreeSet<u16> = corpus_roots(&p.data).into_iter().collect();
    let name = |kt: u16| p.database.types[kt as usize].name.clone();
    let extra: Vec<String> = roots.difference(&expected).map(|k| name(*k)).collect();
    let missing: Vec<String> = expected.difference(&roots).map(|k| name(*k)).collect();
    assert_eq!(
        roots, expected,
        "program_roots must be exactly the corpus's user struct/enum types.\n  \
         extra: {extra:?}\n  missing: {missing:?}"
    );
}
