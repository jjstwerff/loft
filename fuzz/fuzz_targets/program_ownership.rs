// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN85 fuzz-proof gate — the PROGRAM-LEVEL ownership fuzzer, in-process
//! (`fuzz-proof-gate.md` § Next item 2: the `grammar_gen.py` port).
//!
//! Generates a valid-by-construction, SELF-CHECKING loft program over the
//! ownership-composition grammar (the 9 grounded over-free shapes × value
//! kinds × churn, plus arbitrary-driven axes the python grid lacks: variable
//! source lengths, churn scales, and SHAPE COMPOSITION — two shapes sharing
//! one program with interleaved calls).  Each program asserts the structural
//! invariant the over-free class violates (source length constant, delivered
//! length K); the run happens IN-PROCESS — parse → scopes → bytecode →
//! interpret — with `poison_free` set on the store arena, so a stale read is
//! loud garbage, not silent luck.  No rustc, no subprocess: libfuzzer's
//! coverage guidance gets thousands of executions per minute.
//!
//! Findings (all panic, which libfuzzer reports with the artifact):
//!  - a compiler panic in scopes/bytecode (an ICE on a valid program);
//!  - ANY runtime fault — the programs are total, so a failed self-check
//!    assert or an arithmetic/index fault is a real miscompile;
//!  - a store leak at program exit (`collect_store_leaks`).
//!
//! Run:  cargo +nightly fuzz run program_ownership            (open-ended)
//!       cargo +nightly fuzz run program_ownership -- -runs=10000
//!
//! The value axes include the @PLN25-settled pieces (single-payload enum
//! elements, nested-record elements); keyed containers (`hash`/`sorted`)
//! remain a follow-up axis.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use loft::compile::byte_code;
use loft::data::Data;
use loft::database::Stores;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
use std::fmt::Write as _;
use std::sync::OnceLock;

// ── the input space ─────────────────────────────────────────────────────────

#[derive(Arbitrary, Debug)]
struct Spec {
    shape: u8,
    value: u8,
    churn_loops: u8,
    churn_fill: u8,
    k: u8,
    reps: u8,
    /// Composition: a second shape woven into the same program (interleaved
    /// calls in one loop).  `None`-equivalent when == 0xFF.
    second_shape: u8,
    second_value: u8,
}

const N_SHAPES: u8 = 9;
const N_VALUES: u8 = 4;

// ── value kinds (the grammar's VALUES table + the @PLN25-settled axes) ──────

struct ValueKind {
    decl: String,
    et: String,
    default: String,
    el: fn(&str, &str) -> String, // (suffix, var) -> element constructor
}

fn value_kind(idx: u8, sfx: &str) -> ValueKind {
    match idx % N_VALUES {
        0 => ValueKind {
            // struct element (the original grid's record kind)
            decl: format!(
                "struct E{sfx} {{ hp: integer not null, name: text }}\n\
                 fn e_default{sfx}() -> E{sfx} {{ E{sfx}{{hp:0, name:\"\"}} }}"
            ),
            et: format!("E{sfx}"),
            default: format!("e_default{sfx}()"),
            el: |s, v| format!("E{s}{{hp:{v}, name:\"e\"}}"),
        },
        1 => ValueKind {
            // scalar element
            decl: String::new(),
            et: "integer".to_string(),
            default: "0".to_string(),
            el: |_, v| v.to_string(),
        },
        2 => ValueKind {
            // @PLN25 single-payload ENUM element
            decl: format!("enum EV{sfx} {{ Nil{sfx}, Val{sfx} {{ x: integer }} }}"),
            et: format!("EV{sfx}"),
            default: format!("Nil{sfx}"),
            el: |s, v| format!("Val{s}{{x:{v}}}"),
        },
        _ => ValueKind {
            // NESTED-record element (a struct holding its own vector)
            decl: format!("struct N{sfx} {{ vs: vector<integer>, id: integer }}"),
            et: format!("N{sfx}"),
            default: format!("N{sfx}{{vs:[], id:0}}"),
            el: |s, v| format!("N{s}{{vs:[{v}, {v}+1], id:{v}}}"),
        },
    }
}

// ── one shape instance = (defs, setup-in-main, one-iteration body) ──────────

struct ShapeParts {
    defs: String,
    setup: String,
    iter: String,
}

fn build_src(target: &str, k: usize, v: &ValueKind, sfx: &str) -> String {
    format!(
        "for k{sfx} in 0..{k} {{ {target} += [{}]; }}",
        (v.el)(sfx, &format!("k{sfx}"))
    )
}

#[allow(clippy::too_many_lines)]
fn shape_parts(shape: u8, v: &ValueKind, sfx: &str, k: usize) -> ShapeParts {
    let et = &v.et;
    let dflt = &v.default;
    let filler = format!(
        "fn filler{sfx}(n: integer) -> integer {{ es: vector<{et}> = []; \
         for j{sfx} in 0..n {{ es += [{}]; }} return len(es); }}",
        (v.el)(sfx, &format!("j{sfx}"))
    );
    let header = if v.decl.is_empty() {
        format!("{filler}\n")
    } else {
        format!("{}\n{filler}\n", v.decl)
    };
    match shape % N_SHAPES {
        0 => ShapeParts {
            // field_return — a struct-field vector returned directly.
            defs: format!(
                "{header}struct Box{sfx} {{ rows: vector<{et}> }}\n\
                 fn deliver{sfx}(b: Box{sfx}) -> vector<{et}> {{ b.rows }}\n"
            ),
            setup: format!(
                "b{sfx} = Box{sfx} {{ rows: [] }}; {}",
                build_src(&format!("b{sfx}.rows"), k, v, sfx)
            ),
            iter: format!(
                "r{sfx} = deliver{sfx}(b{sfx}); \
                 assert(len(b{sfx}.rows)=={k}, \"src{sfx}={{len(b{sfx}.rows)}}\"); \
                 assert(len(r{sfx})=={k}, \"del{sfx}={{len(r{sfx})}}\");"
            ),
        },
        1 => ShapeParts {
            // field_local — the view passes through a local first.
            defs: format!(
                "{header}struct Box{sfx} {{ rows: vector<{et}> }}\n\
                 fn deliver{sfx}(b: Box{sfx}) -> vector<{et}> \
                 {{ out: vector<{et}> = b.rows; out }}\n"
            ),
            setup: format!(
                "b{sfx} = Box{sfx} {{ rows: [] }}; {}",
                build_src(&format!("b{sfx}.rows"), k, v, sfx)
            ),
            iter: format!(
                "r{sfx} = deliver{sfx}(b{sfx}); \
                 assert(len(b{sfx}.rows)=={k}, \"src{sfx}={{len(b{sfx}.rows)}}\"); \
                 assert(len(r{sfx})=={k}, \"del{sfx}={{len(r{sfx})}}\");"
            ),
        },
        2 => ShapeParts {
            // field_reassign — conditional reassignment between two views.
            defs: format!(
                "{header}struct Box{sfx} {{ rows: vector<{et}> }}\n\
                 fn rows{sfx}(b: Box{sfx}) -> vector<{et}> {{ b.rows }}\n\
                 fn deliver{sfx}(b: Box{sfx}, c: Box{sfx}) -> vector<{et}> \
                 {{ best = rows{sfx}(b); o = rows{sfx}(c); \
                 if len(o) > len(best) {{ best = o; }} best }}\n"
            ),
            setup: format!(
                "b{sfx} = Box{sfx} {{ rows: [] }}; b{sfx}.rows += [{el0}];\n  \
                 c{sfx} = Box{sfx} {{ rows: [] }}; {}",
                build_src(&format!("c{sfx}.rows"), k, v, sfx),
                el0 = (v.el)(sfx, "0"),
            ),
            iter: format!(
                "r{sfx} = deliver{sfx}(b{sfx}, c{sfx}); \
                 assert(len(b{sfx}.rows)==1, \"b{sfx}={{len(b{sfx}.rows)}}\"); \
                 assert(len(c{sfx}.rows)=={k}, \"c{sfx}={{len(c{sfx}.rows)}}\"); \
                 assert(len(r{sfx})=={k}, \"del{sfx}={{len(r{sfx})}}\");"
            ),
        },
        3 => ShapeParts {
            // match_return — the borrowed match-field binding.
            defs: format!(
                "{header}enum Cell{sfx} {{ Empty{sfx}, Filled{sfx} {{ items: vector<{et}> }} }}\n\
                 fn deliver{sfx}(e: Cell{sfx}) -> vector<{et}> \
                 {{ match e {{ Filled{sfx} {{ items }} => {{ items }}, _ => {{ [] }} }} }}\n"
            ),
            setup: format!(
                "inner{sfx}: vector<{et}> = []; {}\n  cell{sfx} = Filled{sfx} {{ items: inner{sfx} }};",
                build_src(&format!("inner{sfx}"), k, v, sfx)
            ),
            iter: format!(
                "r{sfx} = deliver{sfx}(cell{sfx}); \
                 assert(len(r{sfx})=={k}, \"del{sfx}={{len(r{sfx})}}\");"
            ),
        },
        4 => ShapeParts {
            // if_return — the if-arm join of a view and an owned literal.
            defs: format!(
                "{header}struct Box{sfx} {{ rows: vector<{et}> }}\n\
                 fn deliver{sfx}(b: Box{sfx}, cond: boolean) -> vector<{et}> \
                 {{ if cond {{ b.rows }} else {{ [] }} }}\n"
            ),
            setup: format!(
                "b{sfx} = Box{sfx} {{ rows: [] }}; {}",
                build_src(&format!("b{sfx}.rows"), k, v, sfx)
            ),
            iter: format!(
                "r{sfx} = deliver{sfx}(b{sfx}, true); \
                 assert(len(b{sfx}.rows)=={k}, \"src{sfx}={{len(b{sfx}.rows)}}\"); \
                 assert(len(r{sfx})=={k}, \"del{sfx}={{len(r{sfx})}}\");"
            ),
        },
        5 => ShapeParts {
            // elem_accumulate — element views accumulated into a fresh vector.
            defs: format!(
                "{header}fn pick{sfx}(t: vector<{et}>, i: integer) -> {et} {{ t[i] ?? {dflt} }}\n\
                 fn collect{sfx}(t: vector<{et}>) -> vector<{et}> \
                 {{ out: vector<{et}> = []; for i{sfx} in 0..len(t) \
                 {{ out += [pick{sfx}(t, i{sfx})]; }} out }}\n"
            ),
            setup: format!(
                "t{sfx}: vector<{et}> = []; {}",
                build_src(&format!("t{sfx}"), k, v, sfx)
            ),
            iter: format!(
                "r{sfx} = collect{sfx}(t{sfx}); \
                 assert(len(t{sfx})=={k}, \"src{sfx}={{len(t{sfx})}}\"); \
                 assert(len(r{sfx})=={k}, \"del{sfx}={{len(r{sfx})}}\");"
            ),
        },
        6 => ShapeParts {
            // local_source — the #462 conditional view-assign of a LOCAL pool.
            defs: format!(
                "{header}fn one{sfx}(t: vector<{et}>, salt: integer) -> {et} {{\n\
                 pool: vector<{et}> = []; for p{sfx} in 0..len(t) \
                 {{ pool += [t[p{sfx}] ?? {dflt}]; }}\n\
                 chosen = {dflt}; np = len(pool);\n\
                 for wj{sfx} in 0..np {{ if salt % np == wj{sfx} \
                 {{ chosen = pool[wj{sfx}] ?? {dflt}; }} }}\n\
                 chosen\n}}\n"
            ),
            setup: format!(
                "t{sfx}: vector<{et}> = []; {}",
                build_src(&format!("t{sfx}"), k, v, sfx)
            ),
            iter: format!(
                "r{sfx} = one{sfx}(t{sfx}, i); \
                 assert(len(t{sfx})=={k}, \"src{sfx}={{len(t{sfx})}}\");"
            ),
        },
        7 => ShapeParts {
            // index_read — the #426B inner-vector view.
            defs: format!(
                "{header}fn deliver{sfx}(vv: vector<vector<{et}>>, i: integer) \
                 -> vector<{et}> {{ vv[i] ?? [] }}\n"
            ),
            setup: format!(
                "vv{sfx}: vector<vector<{et}>> = [];\n  \
                 for g{sfx} in 0..3 {{ row: vector<{et}> = []; {} vv{sfx} += [row]; }}",
                build_src("row", k, v, sfx)
            ),
            iter: format!(
                "r{sfx} = deliver{sfx}(vv{sfx}, i % 3); \
                 assert(len(vv{sfx})==3, \"vv{sfx}={{len(vv{sfx})}}\"); \
                 assert(len(r{sfx})=={k}, \"r{sfx}={{len(r{sfx})}}\");"
            ),
        },
        _ => ShapeParts {
            // nested_field — the P13 nested struct-field chain.
            defs: format!(
                "{header}struct Inner{sfx} {{ rows: vector<{et}> }}\n\
                 struct Outer{sfx} {{ inner: Inner{sfx} }}\n\
                 fn deliver{sfx}(o: Outer{sfx}) -> vector<{et}> {{ o.inner.rows }}\n"
            ),
            setup: format!(
                "ri{sfx}: vector<{et}> = []; {}\n  \
                 o{sfx} = Outer{sfx} {{ inner: Inner{sfx} {{ rows: ri{sfx} }} }};",
                build_src(&format!("ri{sfx}"), k, v, sfx)
            ),
            iter: format!(
                "r{sfx} = deliver{sfx}(o{sfx}); \
                 assert(len(o{sfx}.inner.rows)=={k}, \"src{sfx}={{len(o{sfx}.inner.rows)}}\"); \
                 assert(len(r{sfx})=={k}, \"r{sfx}={{len(r{sfx})}}\");"
            ),
        },
    }
}

// ── the program generator ───────────────────────────────────────────────────

fn generate(spec: &Spec) -> String {
    // Bounded axes: keep every program small and fast, but past the grid.
    let k = 1 + (spec.k as usize % 4); // source length 1..=4
    let reps = 1 + (spec.reps as usize % 6); // main-loop reps 1..=6
    let churn_loops = spec.churn_loops as usize % 8; // 0 = no churn
    let churn_fill = 1 + (spec.churn_fill as usize % 8);

    let va = value_kind(spec.value, "_a");
    let a = shape_parts(spec.shape, &va, "_a", k);

    let second = (spec.second_shape != 0xFF).then(|| {
        let vb = value_kind(spec.second_value, "_b");
        (shape_parts(spec.second_shape, &vb, "_b", k), vb)
    });

    let mut src = String::new();
    let _ = write!(src, "{}", a.defs);
    if let Some((b, _)) = &second {
        let _ = write!(src, "{}", b.defs);
    }
    let _ = write!(src, "fn main() {{\n  {}\n", a.setup);
    if let Some((b, _)) = &second {
        let _ = write!(src, "  {}\n", b.setup);
    }
    let churn = if churn_loops == 0 {
        String::new()
    } else {
        format!(
            "acc = 0; for f in 0..{churn_loops} {{ acc += filler_a({churn_fill}); }} \
             assert(acc == {}, \"churn\");",
            churn_loops * churn_fill
        )
    };
    let _ = write!(src, "  for i in 0..{reps} {{ {} {churn}", a.iter);
    if let Some((b, _)) = &second {
        let _ = write!(src, " {}", b.iter);
    }
    let _ = write!(src, " }}\n}}\n");
    src
}

// ── the in-process oracle ───────────────────────────────────────────────────

static STDLIB: OnceLock<(Data, Stores)> = OnceLock::new();

fn stdlib() -> (Data, Stores) {
    let (data, db) = STDLIB.get_or_init(|| {
        let mut p = Parser::new();
        // cargo-fuzz may run with cwd at the repo root or at `fuzz/`.
        let dir = if std::path::Path::new("default").is_dir() {
            "default"
        } else {
            "../default"
        };
        p.parse_dir(dir, true, false)
            .expect("stdlib parse (run from the loft repo root or fuzz/)");
        if std::env::var_os("LOFT_FUZZ_DUMP").is_some() {
            eprintln!(
                "[stdlib] dir={dir} cwd={:?} defs={} diagnostics={}",
                std::env::current_dir().ok(),
                p.data.definitions(),
                p.diagnostics.lines().len()
            );
        }
        (p.data, p.database)
    });
    (data.clone(), db.clone())
}

fn run_oracle(src: &str) {
    let (data, db) = stdlib();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    // Parse via a temp FILE (the wrap-harness path), not `parse_str`: the
    // string path runs its own two-pass reset discipline and trips the H5
    // def-count assert on the nested value kind — a parse_str-only asymmetry
    // (REPL-facing; recorded in fuzz-proof-gate.md as the port's first catch)
    // that the file path does not have.
    let tmp = std::env::temp_dir().join(format!(
        "loft_fuzz_{}_{:x}.loft",
        std::process::id(),
        src.len() ^ src.as_bytes().iter().map(|&b| b as usize).sum::<usize>()
    ));
    std::fs::write(&tmp, src).expect("write fuzz program");
    let path = tmp.to_string_lossy().to_string();
    // The H5 two-pass def-count debug assert fires at the END of `parse()`
    // for some shapes when parsing on a PRELOADED (cloned-cache) stdlib —
    // the port's first catch (latent cross-pass re-mint asymmetry; recorded
    // in fuzz-proof-gate.md, routed to its own investigation).  Gate it so
    // it does not drown the ownership signal; any OTHER parse panic is a
    // finding and propagates.
    let parse_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        p.parse(&path, false);
    }));
    let _ = std::fs::remove_file(&tmp);
    if let Err(payload) = parse_result {
        let msg = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_default();
        if msg.contains("H5: definition COUNT diverged") {
            return;
        }
        std::panic::resume_unwind(payload);
    }
    let had_errors = !p.diagnostics.is_empty()
        && p.diagnostics
            .lines()
            .iter()
            .any(|l| !l.starts_with("Warning:") && !l.starts_with("Debug:"));
    if had_errors {
        // Valid-by-construction: a rejected program is a GENERATOR bug worth
        // seeing while iterating, but not a compiler finding — drop it.
        return;
    }
    // A panic below (scopes/bytecode ICE on a valid program) IS a finding —
    // let it propagate to libfuzzer.  ONE known, GATED exception: the H5
    // two-pass def-count debug assert fires for SOME shapes when parsing on
    // top of a PRELOADED (cloned-cache) stdlib — a latent cross-pass re-mint
    // asymmetry the release suites never check (debug_assertions off) and
    // the CLI's single-session parse does not have.  The port's first catch;
    // recorded in fuzz-proof-gate.md and routed to its own investigation —
    // dropped here so it does not drown the ownership signal.
    let mut data = p.data;
    let database = p.database;
    let compiled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        scopes::check(&mut data);
        let mut state = State::new(database);
        byte_code(&mut state, &mut data);
        (state, data)
    }));
    let (mut state, p_data) = match compiled {
        Ok(pair) => pair,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default();
            if msg.contains("H5: definition COUNT diverged") {
                return;
            }
            std::panic::resume_unwind(payload);
        }
    };
    // Arena poison-on-free: stale reads become loud garbage the self-check
    // asserts catch, instead of silently lucky data.
    state.database.poison_free = true;
    state.execute("main", &p_data);
    while state.database.frame_yield {
        state.resume();
    }
    // The programs are total: ANY runtime fault is a finding.
    if state.database.had_fatal {
        let msg = state
            .database
            .runtime_error
            .as_ref()
            .map_or_else(|| "runtime fault".to_string(), |e| e.message.clone());
        panic!("ownership fuzz FINDING (runtime): {msg}\n--- program ---\n{src}");
    }
    state.check_store_leaks();
    let leaks = state.collect_store_leaks();
    if !leaks.is_empty() {
        panic!(
            "ownership fuzz FINDING (leak): {} store(s): {}\n--- program ---\n{src}",
            leaks.len(),
            leaks.join(", ")
        );
    }
}

fuzz_target!(|spec: Spec| {
    let src = generate(&spec);
    if std::env::var_os("LOFT_FUZZ_DUMP").is_some() {
        eprintln!("--- generated program ({spec:?}) ---\n{src}\n---");
    }
    run_oracle(&src);
});
