// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1313 — `(N-Store)` for the HEAP half of the rule.
//!
//! `formal/types.md` states the default for every type: *"Storage is non-null by default: a
//! binding, field, or `vector` element of type `τ` never holds `null` — `τ?` is the only way a
//! slot admits it."*  @PLN25's DN1 landed that for the SCALARS, and `n_store_violation` gated
//! the whole branch on `is_non_null_scalar` — so a bare `null` into a non-null REFERENCE,
//! collection or struct-enum passed in silence at the four positions where the scalar twin
//! warns.  `keys::callarg_nstore_enabled`'s own doc already described the intended split as
//! *"a non-narrow scalar/heap param WARNS"*, so the heap half was specified and never wired in.
//!
//! Two halves need scoring here and only one of them has a corpus channel.  A `.loft` guard can
//! declare a notice it EXPECTS (`@EXPECT_WARNING`), but it cannot assert that a notice must NOT
//! fire — so every negative control below is a COUNT of the notices on stderr.  That is also why
//! `make falsify` cannot score this change at all: the values and the exit codes do not move.
//!
//! It only ever warns.  There is no narrow heap width to run out of room the way a `u8` does,
//! and loft#1232 settled the compatibility half — reporting where there was silence is a strict
//! gain, refusing what a shipped package already compiles is the break the freeze forbids.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Run `body` on `backend`, with the heap half of `(N-Store)` on or off.
/// Returns `(exit-ok, stdout, stderr)`.  `tag` keeps the temp script unique across the
/// parallel tests.
fn run(body: &str, backend: &str, heap_on: bool, tag: &str) -> (bool, String, String) {
    let script = std::env::temp_dir().join(format!("loft_hns_{}_{tag}.loft", std::process::id()));
    std::fs::write(&script, body).expect("write script");
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend).arg(&script).env("LOFT_TIMEOUT", "120");
    if heap_on {
        cmd.env_remove("LOFT_NO_HEAP_NSTORE");
    } else {
        cmd.env("LOFT_NO_HEAP_NSTORE", "1");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Notices from the HEAP half.  The DN3 message ("a nullable `τ?` is stored into … of the
/// non-null type `X`") shares the "non-null type" wording, so the count keys on the bare-`null`
/// opening as well — counting one message by a substring the other also carries is how a
/// negative control reads green while the notice it denies is firing.
fn heap_notices(err: &str) -> usize {
    err.lines()
        .filter(|l| {
            l.contains("`null` is stored into")
                && l.contains("of the non-null type `")
                && !l.contains("scalar type")
        })
        .count()
}

/// Notices from the SCALAR half, which must not move.
fn scalar_notices(err: &str) -> usize {
    err.lines()
        .filter(|l| {
            l.contains("`null` is stored into") && l.contains("of the non-null scalar type `")
        })
        .count()
}

const BACKENDS: [&str; 2] = ["--interpret", "--native"];

// ---------------------------------------------------------------- the four positions

/// One source per position, each storing a bare `null` into a non-null STRUCT target.
/// `{what}` differs per position in the message, so these also say the existing wording
/// reaches the heap half unchanged.
const POSITIONS: [(&str, &str); 4] = [
    (
        "return",
        "struct It { v: integer }\n\
         fn f(k: integer) -> It { if k > 0 { return null; } It { v: 1 } }\n\
         fn main() { print(\"{f(9) == null}\\n\"); }\n",
    ),
    (
        "field",
        "struct It { v: integer }\n\
         struct Box { i: It }\n\
         fn main() { b = Box { i: null }; print(\"{b.i == null}\\n\"); }\n",
    ),
    (
        "vector element",
        "struct It { v: integer }\n\
         fn main() { v: vector<It> = [null]; print(\"{len(v)}\\n\"); }\n",
    ),
    (
        "call argument",
        "struct It { v: integer }\n\
         fn g(i: It) -> boolean { i == null }\n\
         fn main() { print(\"{g(null)}\\n\"); }\n",
    ),
];

#[test]
fn every_position_that_warns_for_a_scalar_warns_for_a_record() {
    for (position, src) in POSITIONS {
        for backend in BACKENDS {
            let tag = format!("pos_{}_{}", position.replace(' ', "_"), &backend[2..]);
            let (ok, _out, err) = run(src, backend, true, &tag);
            assert_eq!(
                heap_notices(&err),
                1,
                "a null into a non-null record at the {position} position must warn exactly once \
                 on {backend}\n{err}"
            );
            // It WARNS: the store proceeds and the program runs.  An error here would be a new
            // refusal on a shape that compiles today, which the freeze forbids.
            assert!(
                ok,
                "the {position} position must still RUN on {backend} — the notice is a warning, \
                 not a refusal\n{err}"
            );
        }
    }
}

/// The switch is the control that attributes the notices above to this change and nothing else.
#[test]
fn the_opt_out_silences_every_position() {
    for (position, src) in POSITIONS {
        let tag = format!("off_{}", position.replace(' ', "_"));
        let (ok, _out, err) = run(src, "--interpret", false, &tag);
        assert_eq!(
            heap_notices(&err),
            0,
            "LOFT_NO_HEAP_NSTORE must silence the {position} position\n{err}"
        );
        assert!(
            ok,
            "the opt-out must not change whether the program runs\n{err}"
        );
    }
}

/// `LOFT_NO_NULLFLOW` opts out of the whole @PLN102 model, and its documented meaning is the
/// pre-Phase-1 behaviour: a uniform hard ERROR for the scalars.  The heap half had no behaviour
/// there at all — it was silent — so falling through to that error would hand the opt-out a
/// refusal this branch never carried, which is the one outcome the freeze forbids.  Silence is
/// what "give me the old model" has to mean here.
#[test]
fn opting_out_of_nullflow_restores_silence_not_a_refusal() {
    let src = "struct It { v: integer }\nfn f() -> It { return null; }\n               fn main() { print(\"{f() == null}\\n\"); }\n";
    let script = std::env::temp_dir().join(format!("loft_hns_{}_noflow.loft", std::process::id()));
    std::fs::write(&script, src).expect("write script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_NULLFLOW", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        heap_notices(&err),
        0,
        "LOFT_NO_NULLFLOW must return the heap half to silence\n{err}"
    );
    assert!(
        out.status.success(),
        "and must not refuse it — the heap branch never had an error to fall back to\n{err}"
    );
}

// ---------------------------------------------------------------- the heap KINDS

/// `is_dbref` carries the full set, and its own doc records how a hand-spelled copy drifts: the
/// three obvious kinds get written and the five KEYED collections are forgotten, because they
/// are reached by key and do not look like references at the call site.  So a keyed cell is here
/// on purpose — it is the one a re-implementation of this predicate would lose.
#[test]
fn every_heap_kind_is_covered_including_a_keyed_collection() {
    let kinds: [(&str, &str); 4] = [
        (
            "record",
            "struct It { v: integer }\nfn f() -> It { return null; }\n\
             fn main() { print(\"{f() == null}\\n\"); }\n",
        ),
        (
            "vector",
            "fn f() -> vector<integer> { return null; }\n\
             fn main() { print(\"{f() == null}\\n\"); }\n",
        ),
        (
            "struct-enum",
            "enum Sh { A { k: integer }, B { k: integer } }\nfn f() -> Sh { return null; }\n\
             fn main() { print(\"{f() == null}\\n\"); }\n",
        ),
        (
            "keyed hash",
            "struct E { id: integer }\nfn f() -> hash<E[id]> { return null; }\n\
             fn main() { print(\"{f() == null}\\n\"); }\n",
        ),
    ];
    for (kind, src) in kinds {
        for backend in BACKENDS {
            let tag = format!("kind_{}_{}", kind.replace([' ', '-'], "_"), &backend[2..]);
            let (ok, _out, err) = run(src, backend, true, &tag);
            assert_eq!(
                heap_notices(&err),
                1,
                "a null into a non-null {kind} return must warn on {backend}\n{err}"
            );
            assert!(ok, "the {kind} cell must still run on {backend}\n{err}");
        }
    }
}

// ---------------------------------------------------------------- the negative controls
//
// These are the half a `.loft` guard cannot express.  Each names a slot that IS allowed to hold
// null, so a notice there would be the diagnostic firing on correct code — the failure mode that
// decides whether a lint is worth shipping.

#[test]
fn a_nullable_target_is_silent() {
    let cells: [(&str, &str); 4] = [
        (
            "record return",
            "struct It { v: integer }\nfn f() -> It? { return null; }\n\
             fn main() { print(\"{f() == null}\\n\"); }\n",
        ),
        (
            "collection return",
            "fn f() -> vector<integer>? { return null; }\n\
             fn main() { print(\"{f() == null}\\n\"); }\n",
        ),
        (
            "nullable field",
            "struct It { v: integer }\nstruct Box { i: It? }\n\
             fn main() { b = Box { i: null }; print(\"{b.i == null}\\n\"); }\n",
        ),
        (
            "nullable parameter",
            "struct It { v: integer }\nfn g(i: It?) -> boolean { i == null }\n\
             fn main() { print(\"{g(null)}\\n\"); }\n",
        ),
    ];
    for (cell, src) in cells {
        for backend in BACKENDS {
            let tag = format!("nn_{}_{}", cell.replace(' ', "_"), &backend[2..]);
            let (ok, _out, err) = run(src, backend, true, &tag);
            assert_eq!(
                heap_notices(&err),
                0,
                "a `τ?` {cell} declares that it admits null — warning there would fire on correct \
                 code ({backend})\n{err}"
            );
            assert!(ok, "the {cell} control must run on {backend}\n{err}");
        }
    }
}

/// A `reference<T>` field on a reference CYCLE back to its own struct warns like any other
/// field, and the cure it names is one the author can actually write.
///
/// It was SILENT while `struct Node { next: reference<Node>? }` failed layout validation: the
/// linked-list terminator had to be a bare `null` in a non-null slot, so the notice would have
/// named a cure that does not compile, and a diagnostic whose cure is rejected is worse than
/// silence.  loft#1316 gave the field its nullable spelling — `@FR-L-Null` keeps a pointer's own
/// bytes and spends `nullref` on absence — so the suppression went with it.
///
/// The cure is checked as TEXT, not just for presence.  `Type::name` renders a pointer field as
/// the bare struct name, which would print `Node?` — and `next: Node?` is the INLINE tagged form
/// (`@FR-L-Null-Tag`), which on a self-referencing struct has no finite size and does not
/// compile.  Naming `reference<Node>?` is the whole point of the flip.
#[test]
fn a_cyclic_reference_field_warns_and_names_a_cure_that_compiles() {
    let cells: [(&str, &str, &str); 2] = [
        (
            "direct self-reference",
            "Node",
            "struct Node { value: integer, next: reference<Node> }
             fn main() { c = Node { value: 4, next: null }; print(\"{c.value}\\n\"); }
",
        ),
        (
            "mutual reference",
            "B",
            "struct A { v: integer, b: reference<B> }
             struct B { v: integer, a: reference<A> }
             fn main() { x = A { v: 1, b: null }; print(\"{x.v}\\n\"); }
",
        ),
    ];
    for (cell, cure, src) in cells {
        for backend in BACKENDS {
            let tag = format!("cyc_{}_{}", cell.replace([' ', '-'], "_"), &backend[2..]);
            let (ok, _out, err) = run(src, backend, true, &tag);
            assert_eq!(
                heap_notices(&err),
                1,
                "a {cell} field now has a nullable spelling, so it warns like any other \
                 ({backend})\n{err}"
            );
            assert!(
                err.contains(&format!("declare it `reference<{cure}>?`")),
                "the cure must name the FIELD's own type, not the bare struct — a `{cure}?` \
                 field is the inline tagged form and does not compile here ({backend})\n{err}"
            );
            assert!(ok, "the {cell} cell must still run on {backend}\n{err}");
        }
    }
}

/// Every position warns, and each one names its own type back.
///
/// The cyclic field above used to be the exception; now that it is not, what is left to keep
/// straight is which SPELLING the cure is given in.  A POINTER field and an EMBEDDED one are
/// one `Type::Reference` apart only by the `u16::MAX` share marker, and their cures are not
/// interchangeable: `reference<Leaf>?` keeps the pointer (`@FR-L-Null`), `Leaf?` is the inline
/// tagged record (`@FR-L-Null-Tag`).  Handing a pointer field the second compiles and silently
/// swaps sharing for a copy, which is the defect loft#1316 closed — so the message naming the
/// wrong one would re-open it in prose.
#[test]
fn each_position_names_its_own_type_as_the_cure() {
    // An ACYCLIC `reference<T>` field: a pointer, so its cure carries the former.
    let acyclic = "struct Leaf { v: integer }
                   struct Holder { l: reference<Leaf> }
                   fn main() { h = Holder { l: null }; print(\"{h.l == null}\\n\"); }
";
    // An EMBEDDED struct field: no marker, no former — the bare name IS its type.
    let embedded = "struct Leaf { v: integer }
                    struct Emb { l: Leaf }
                    fn main() { e = Emb { l: null }; print(\"{e.l == null}\\n\"); }
";
    // A RETURN of a cyclic type, which was never in the exclusion: `-> Node?` always compiled.
    let cyclic_return = "struct Node { value: integer, next: reference<Node> }
                         fn f() -> Node { return null; }
                         fn main() { print(\"{f() == null}\\n\"); }
";
    for (cell, cure, src) in [
        ("acyclic pointer field", "reference<Leaf>?", acyclic),
        ("embedded struct field", "Leaf?", embedded),
        ("cyclic type's return", "Node?", cyclic_return),
    ] {
        for backend in BACKENDS {
            let tag = format!("disc_{}_{}", cell.replace([' ', '\''], "_"), &backend[2..]);
            let (ok, _out, err) = run(src, backend, true, &tag);
            assert_eq!(
                heap_notices(&err),
                1,
                "the {cell} has a nullable spelling and must still warn ({backend})\n{err}"
            );
            assert!(
                err.contains(&format!("declare it `{cure}`")),
                "the {cell} must be cured in its OWN spelling, `{cure}` ({backend})\n{err}"
            );
            assert!(ok, "the {cell} cell must still run on {backend}\n{err}");
        }
    }
}

/// The synthetic `__nullable<S>` is the INLINE spelling of `S?` — a struct held in a vector
/// element or a field slot rather than behind a handle.  It is a `Type::Enum(_, true, _)` and so
/// reaches `is_dbref`, which is why the wrapper is excluded explicitly: it is exactly as nullable
/// as the `?` it stands for, and reading it as a non-null target would warn on `vector<It?>`.
#[test]
fn the_inline_nullable_wrapper_is_silent() {
    let src = "struct It { v: integer }\n\
               fn main() { v: vector<It?> = [null]; print(\"{len(v)}\\n\"); }\n";
    for backend in BACKENDS {
        let (ok, _out, err) = run(src, backend, true, &format!("wrap_{}", &backend[2..]));
        assert_eq!(
            heap_notices(&err),
            0,
            "a `vector<It?>` element admits null by declaration ({backend})\n{err}"
        );
        assert!(ok, "the wrapper control must run on {backend}\n{err}");
    }
}

/// A non-null value into a non-null heap slot is the ordinary case, and by far the commonest.
/// If this warned, the notice would be unusable whatever it said about the null cells.
#[test]
fn a_present_value_into_a_non_null_slot_is_silent() {
    let src = "struct It { v: integer }\n\
               struct Box { i: It }\n\
               fn f() -> It { It { v: 7 } }\n\
               fn main() { b = Box { i: f() }; print(\"{b.i.v}\\n\"); }\n";
    for backend in BACKENDS {
        let (ok, out, err) = run(src, backend, true, &format!("present_{}", &backend[2..]));
        assert_eq!(
            heap_notices(&err),
            0,
            "the ordinary non-null store must stay silent ({backend})\n{err}"
        );
        assert!(ok, "the present-value control must run on {backend}\n{err}");
        assert!(
            out.contains('7'),
            "and must still compute its value ({backend}): {out}"
        );
    }
}

// ---------------------------------------------------------------- the scalar half is unmoved

/// The scalar wording is a public string that fixtures pin.  The heap half DROPS the word
/// `scalar` from the same message rather than adding a second one, so this says the scalar
/// spelling still arrives exactly as before — and that the two halves stayed one diagnostic.
#[test]
fn the_scalar_wording_is_unchanged() {
    let src = "fn f(k: integer) -> integer { if k > 0 { return null; } 1 }\n\
               fn main() { print(\"{f(9) == null}\\n\"); }\n";
    for backend in BACKENDS {
        let (ok, _out, err) = run(src, backend, true, &format!("scalar_{}", &backend[2..]));
        assert_eq!(
            scalar_notices(&err),
            1,
            "the scalar notice must still fire, with its own wording ({backend})\n{err}"
        );
        assert_eq!(
            heap_notices(&err),
            0,
            "and must not be counted as a heap notice ({backend})\n{err}"
        );
        assert!(ok, "the scalar cell still runs on {backend}\n{err}");
        assert!(
            err.contains("declare it `integer?` to make that explicit"),
            "the cure the message names is part of the wording ({backend})\n{err}"
        );
    }
}
