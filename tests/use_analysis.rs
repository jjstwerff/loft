// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN25 — the USE-analysis copy-vs-borrow VERDICT, tested in isolation via the
// behaviour-neutral `LOFT_MATERIALIZE_DUMP` (it prints one `MAT fn=… verdict=…` line
// per recognised vector-copy binding and changes no codegen). This is the dependable
// layer the elision builds on; the test pins the verdict per boundary cell so we can
// iterate the analysis without wiring it into emission. Design:
// doc/claude/plans/25-nullable-sequences/{use-analysis-prework,materialization-algorithm}-design.md.

use std::io::Write;
use std::process::Command;

/// Run the loft binary on a source string with the verdict dump on; return stderr.
fn dump(src: &str) -> String {
    use std::hash::{Hash, Hasher};
    let dir = std::env::temp_dir().join("loft_use_analysis");
    std::fs::create_dir_all(&dir).expect("probe dir");
    // Key the file on the source so parallel tests don't clobber each other's probe.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    let path = dir.join(format!("probe_{:016x}.loft", h.finish()));
    let mut f = std::fs::File::create(&path).expect("write probe");
    f.write_all(src.as_bytes()).expect("write probe body");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret", "--check"])
        .arg(&path)
        .env("LOFT_MATERIALIZE_DUMP", "1")
        .env("LOFT_NO_CACHE", "1")
        // The oracle assertions are hand-computed against the RAW shapes; the
        // (now default-on) match-return synthesis rewrites the borrowed arm to
        // an owned copy (`_mvcopy_N`) before the oracle reads it.  Opt out so
        // the ground truth stays expressible; the synthesis's own observable
        // effect is pinned separately (join_own_match_return_strips_the_borrow).
        .env("LOFT_NO_JOIN_OWN", "1")
        .output()
        .expect("spawn loft");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Assert the verdict line for a function contains the expected verdict.
fn assert_verdict(stderr: &str, func: &str, want_verdict: &str) {
    let line = stderr
        .lines()
        .find(|l| l.contains(&format!("fn=n_{func} ")))
        .unwrap_or_else(|| panic!("no MAT line for {func}; dump:\n{stderr}"));
    assert!(
        line.contains(&format!("verdict={want_verdict}")),
        "{func}: expected {want_verdict}, got: {line}"
    );
}

/// Assert the Stage-1 ownership classification of a function's RETURN value
/// (`OWN fn=n_<func> return=<Owned|Borrowed|Join>`).
fn assert_own_return(stderr: &str, func: &str, want: &str) {
    let needle = format!("OWN fn=n_{func} return=");
    let line = stderr
        .lines()
        .find(|l| l.contains(&needle))
        .unwrap_or_else(|| panic!("no OWN return line for {func}; dump:\n{stderr}"));
    assert!(
        line.contains(&format!("return={want}")),
        "{func} return: expected {want}, got: {line}"
    );
}

/// Assert an owned-slot REASSIGNMENT of `var` in `func` carries the expected
/// prior/rhs classes (`OWN fn=n_<func> reassign v=…(var) prior=… rhs=…`).
fn assert_reassign(stderr: &str, func: &str, var: &str, prior: &str, rhs: &str) {
    let head = format!("OWN fn=n_{func} reassign ");
    let line = stderr
        .lines()
        .find(|l| l.starts_with(&head) && l.contains(&format!("({var})")))
        .unwrap_or_else(|| panic!("no OWN reassign line for {func}/{var}; dump:\n{stderr}"));
    assert!(
        line.contains(&format!("prior={prior} rhs={rhs}")),
        "{func}/{var}: expected prior={prior} rhs={rhs}, got: {line}"
    );
}

/// Assert NO owned-slot reassignment is reported for `func` (the WORKING
/// discriminator — a single-def slot carries no displaced owned store).
fn assert_no_reassign(stderr: &str, func: &str) {
    let head = format!("OWN fn=n_{func} reassign ");
    if let Some(line) = stderr.lines().find(|l| l.starts_with(&head)) {
        panic!("{func}: expected no reassign site, got: {line}");
    }
}

/// Assert a Stage-1.5 free SITE of the given kind in `func` carries the expected
/// class and borrow base (`OWN fn=n_<func> free kind=<kind> … class=<class> base=<base>`).
fn assert_free_site(stderr: &str, func: &str, kind: &str, class: &str, base: &str) {
    let head = format!("OWN fn=n_{func} free kind={kind} ");
    let line = stderr
        .lines()
        .find(|l| l.starts_with(&head))
        .unwrap_or_else(|| panic!("no OWN free {kind} line for {func}; dump:\n{stderr}"));
    assert!(
        line.contains(&format!("class={class}")) && line.ends_with(&format!("base={base}")),
        "{func} free {kind}: expected class={class} base={base}, got: {line}"
    );
}

/// Assert NO free site is reported for `func` (a clean shape has no over-free site).
fn assert_no_free_site(stderr: &str, func: &str) {
    let head = format!("OWN fn=n_{func} free ");
    if let Some(line) = stderr.lines().find(|l| l.starts_with(&head)) {
        panic!("{func}: expected no free site, got: {line}");
    }
}

const SRC: &str = r#"
struct Sim { tiles: vector<integer> not null, walls: vector<integer> not null }

// read-only local off a param field -> BORROW (the tile_at / edge_wall_raw shape)
fn tile_at(s: Sim, i: integer) -> integer {
  t = s.tiles;
  if i < len(t) { t[i] ?? 1 } else { 1 }
}
fn edge_at(s: Sim, i: integer) -> integer {
  w = s.walls;
  if i < len(w) { w[i] ?? 0 } else { 0 }
}

// the local is element-mutated -> COPY
fn mutate(s: Sim, i: integer) -> integer {
  t = s.tiles;
  t[0] = 9;
  t[i] ?? 0
}

// the source is a LOCAL (not a parameter) — Tier-0 cannot prove its store
// outlives the local, so it stays a COPY (widened in a later tier)
fn local_src(i: integer) -> integer {
  tmp = Sim { tiles: [4, 5, 6], walls: [0] };
  u = tmp.tiles;
  u[i] ?? 0
}

// the local is handed to a user function (may mutate / escape) -> COPY
fn sink(x: vector<integer>) -> integer { len(x) }
fn to_user(s: Sim) -> integer {
  w = s.tiles;
  sink(w)
}

// the SOURCE is read by a non-mutating callee -> still BORROW. This is the
// interprocedural precision the shared `find_written_vars` buys: a purely
// intraprocedural source check would have to assume `peek` might mutate `s`.
fn peek(s: Sim) -> integer { len(s.tiles) }
fn via_peek(s: Sim, i: integer) -> integer {
  u = s.tiles;
  d = peek(s);
  u[i] ?? d
}

// the SOURCE is mutated through a callee while the local is live -> COPY
// (caught only because the mutation analysis is interprocedural).
fn poke(s: &Sim) { s.tiles[0] = 1; }
fn via_poke(s: Sim, i: integer) -> integer {
  u = s.tiles;
  poke(s);
  u[i] ?? 0
}

fn main() {
  s = Sim { tiles: [1, 2, 3], walls: [0, 0, 0] };
  print(
    "{tile_at(s, 1)} {edge_at(s, 1)} {mutate(s, 1)} {local_src(1)} {to_user(s)} {via_peek(s, 1)} {via_poke(s, 1)}\n"
  );
}
"#;

#[test]
fn verdicts_per_boundary_cell() {
    let stderr = dump(SRC);
    // read-only param-field accessors elide to a borrow
    assert_verdict(&stderr, "tile_at", "Borrow");
    assert_verdict(&stderr, "edge_at", "Borrow");
    // every divergence event forces the conservative copy
    assert_verdict(&stderr, "mutate", "Copy"); // D1: local written
    assert_verdict(&stderr, "local_src", "Copy"); // D3: non-param source (Tier-0 limit)
    assert_verdict(&stderr, "to_user", "Copy"); // D3/escape: passed to a user fn
    // interprocedural source-mutation (¬D2) via the shared find_written_vars
    assert_verdict(&stderr, "via_peek", "Borrow"); // callee only reads the source
    assert_verdict(&stderr, "via_poke", "Copy"); // callee mutates the source
}

// ── Tier 1 (LOFT_ELIDE_T1): read-only LOCAL source, ordering-proven ────────────

/// Run with the verdict dump on at an explicit elision tier (env-selected).
fn dump_at_tier(src: &str, tier: u8) -> String {
    use std::hash::{Hash, Hasher};
    let dir = std::env::temp_dir().join("loft_use_analysis_t1");
    std::fs::create_dir_all(&dir).expect("probe dir");
    // Key the file on (src, tier) so parallel tests don't clobber each other.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    let path = dir.join(format!("probe_t{tier}_{:016x}.loft", h.finish()));
    let mut f = std::fs::File::create(&path).expect("write probe");
    f.write_all(src.as_bytes()).expect("write probe body");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.args(["--interpret", "--check"])
        .arg(&path)
        .env("LOFT_MATERIALIZE_DUMP", "1")
        .env("LOFT_NO_CACHE", "1")
        // Same raw-shape contract as `dump` — see the note there.
        .env("LOFT_NO_JOIN_OWN", "1");
    if tier >= 1 {
        cmd.env("LOFT_ELIDE_T1", "1");
    }
    let out = cmd.output().expect("spawn loft");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const T1_SRC: &str = r#"
struct G { c: vector<integer> not null }
fn bump(g: &G) { g.c[0] = 99; }
fn ro_local() -> integer { x = G { c: [10,20,30] }; v = x.c; v[1] ?? -1 }
fn d2_mut_src() -> integer { x = G { c: [10,20,30] }; v = x.c; x.c[0] = 99; v[0] ?? -1 }
fn callee_mut() -> integer { x = G { c: [10,20,30] }; v = x.c; bump(x); v[0] ?? -1 }
fn rebind() -> integer { x = G { c: [10,20,30] }; v = x.c; x = G { c: [7,8,9] }; v[0] ?? -1 }
fn in_loop() -> integer {
  acc = 0;
  for i in 0..3 { x = G { c: [10,20,30] }; v = x.c; acc += v[0] ?? 0; }
  acc
}
fn main() { print("{ro_local()} {d2_mut_src()} {callee_mut()} {rebind()} {in_loop()}\n"); }
"#;

/// The tier GATE: at Tier 0 a read-only local source stays Copy; Tier 1 flips
/// exactly that case to Borrow and nothing else.
#[test]
fn tier1_gate_flips_only_readonly_local_source() {
    // Tier 0 (default) — every local-source case is Copy.
    let t0 = dump_at_tier(T1_SRC, 0);
    for f in ["ro_local", "d2_mut_src", "callee_mut", "rebind", "in_loop"] {
        assert_verdict(&t0, f, "Copy");
    }
    // Tier 1 — ONLY the read-only local source borrows; the divergence /
    // adversarial cells stay Copy (¬D2 via callee, rebind, loop back-edge).
    let t1 = dump_at_tier(T1_SRC, 1);
    assert_verdict(&t1, "ro_local", "Borrow");
    assert_verdict(&t1, "d2_mut_src", "Copy"); // source mutated after the fill
    assert_verdict(&t1, "callee_mut", "Copy"); // mutated through a callee
    assert_verdict(&t1, "rebind", "Copy"); // source reconstructed after the fill
    assert_verdict(&t1, "in_loop", "Copy"); // copy-fill under a loop back-edge
}

/// Cross-check: the existing `local_src` boundary cell (Copy at Tier 0) is exactly
/// a Tier-1 shape and must flip to Borrow under the flag.
#[test]
fn tier1_flips_existing_local_src_cell() {
    assert_verdict(&dump_at_tier(SRC, 0), "local_src", "Copy");
    assert_verdict(&dump_at_tier(SRC, 1), "local_src", "Borrow");
}

/// RUNTIME guard: with Tier 1 actually wired into the elision (the flag makes
/// `elide_borrows` consume the new freeable-LOCAL-source plans), the full matrix —
/// safe borrows + unsafe copies — stays value-correct on BOTH backends with no
/// leak. A wrong borrow on a freeable local would be a UAF / wrong value here.
#[test]
fn tier1_runtime_correct_both_backends() {
    let script = "tests/scripts/85-tier1-local-source-matrix.loft";
    for backend in ["--interpret", "--native"] {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args([backend, script])
            .env("LOFT_ELIDE_T1", "1")
            .env("LOFT_STORES", "warn")
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("spawn loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stdout.contains("tier1-local-source-matrix ok"),
            "{backend} tier1 matrix failed:\nstdout:{stdout}\nstderr:{stderr}"
        );
        assert!(
            !stderr.contains("not freed"),
            "{backend} tier1 matrix leaked a store:\n{stderr}"
        );
    }
}

// ── @PLN85 Stage 1: the Owned|Borrowed|Join ownership classification ───────────
// The over-free class needs ONE carried fact — for a value escaping into an owned
// position (return / reassign / append), is its store Owned, Borrowed, or a runtime
// Join? This pins that classification (still INERT — printed under the same dump,
// wired into no codegen) on the three live over-free shapes + the FIXED field-view
// family. Design + boundary map:
// doc/claude/plans/85-store-lifetime-retirement/{over-free-class-study,NEXT-SESSION-join-ownership-analysis}.md

const OWN_SRC: &str = r#"
struct M { hp: integer not null, name: text }
fn dflt() -> M { M{hp:0, name:""} }

// elem_accumulate ROOT: `t[i] ?? dflt()` is owned on the dflt() arm, borrowed on
// the t[i] arm — a runtime JOIN the flattened return dep (M["t"]) hides. The same
// `pick` underlies BOTH the source-free (UAF) and the all-owned (CLEAN) repros:
// they are statically IDENTICAL — Join — and the fix (materialise the borrow arm
// to owned) makes both correct regardless of which branch runs.
fn pick(t: vector<M>, i: integer) -> M { t[i] ?? dflt() }

// elem_accumulate FREE SITE: `out += [pick(t,i)]` lowers to an OpCopyRecord with
// the 0x8000 source-free bit on pick's Join return — the AppendSource site.
fn collect(t: vector<M>) -> vector<M> { out: vector<M> = []; for i in 0..len(t) { out += [pick(t, i)]; } out }

// local_source ROOT (#462 leak): `chosen` first OWNS dflt(), then is reassigned to
// a JOIN — the displaced owned store leaks. The over-free shape = prior Owned, rhs
// Join. The return itself is Owned (a materialized_view_return mints a fresh store).
fn pick_cond(t: vector<M>, salt: integer) -> M {
  pool: vector<M> = []; for p in 0..len(t) { pool += [t[p] ?? dflt()]; }
  chosen = dflt();
  np = len(pool);
  for wj in 0..np { if salt % np == wj { chosen = pool[wj] ?? dflt(); } }
  chosen
}
// WORKING discriminator: a single-def `chosen` displaces no owned store → no
// reassign site → no leak.
fn pick_uncond(t: vector<M>, idx: integer) -> M {
  pool: vector<M> = []; for p in 0..len(t) { pool += [t[p] ?? dflt()]; }
  chosen = pool[idx] ?? dflt();
  chosen
}

// match_return ROOT: a match arm delivers a borrowed enum-field view; the other
// arm an empty owned vector → the return is a JOIN.
struct E { hp: integer not null, name: text }
enum Cell { Empty, Filled { items: vector<E> } }
fn deliver(e: Cell) -> vector<E> { match e { Filled { items } => { items }, _ => { [] } } }

// field-view family (all FIXED / clean on the boundary map): a struct-field view
// and a whole-arg view are plain BORROWS of a parameter — safe to return.
struct Box { items: vector<integer> }
fn getf(b: Box) -> vector<integer> { b.items }
fn whole(v: vector<integer>) -> vector<integer> { v }
// a nested-field view roots its base through the projection chain to `o`.
struct Inner { rows: vector<E> }
struct Outer { inner: Inner }
fn nested(o: Outer) -> vector<E> { o.inner.rows }

fn main() {
  t: vector<M> = []; for k in 0..3 { t += [M{hp:k, name:"m"}]; }
  a = pick(t, 0); b = pick_cond(t, 1); c = pick_uncond(t, 0); cc = collect(t);
  cell = Filled { items: [] }; d = deliver(cell);
  bx = Box{ items: [1, 2, 3] }; r = getf(bx); w = whole([4, 5]);
  oo = Outer{ inner: Inner{ rows: [] } }; nn = nested(oo);
  print("{a.hp} {b.hp} {c.hp} {len(cc)} {len(d)} {len(r)} {len(w)} {len(nn)}\n");
}
"#;

/// The Stage-1 ownership fact, pinned per over-free shape. This is the VERDICT the
/// Stage-3 free sites will read; the test gates the analysis in isolation (nothing
/// emits off it yet), so it can be iterated before any codegen change.
#[test]
fn ownership_classifies_the_over_free_shapes() {
    let stderr = dump(OWN_SRC);

    // dflt mints a fresh struct -> Owned (the owned arm of every `??` join below).
    assert_own_return(&stderr, "dflt", "Owned");

    // elem_accumulate: `t[i] ?? dflt()` is a runtime Join. (The CLEAN/all-owned
    // repro uses this SAME `pick` — statically Join too; the static fact cannot
    // and need not distinguish the runtime branch, and Join-awareness covers both.)
    assert_own_return(&stderr, "pick", "Join");

    // local_source: the displaced-owned-store leak is `chosen` reassigned from an
    // OWNED init to a JOIN. The WORKING (single-def) form has no such site.
    assert_reassign(&stderr, "pick_cond", "chosen", "Owned", "Join");
    assert_no_reassign(&stderr, "pick_uncond");
    // pick_cond's RETURN is the fresh materialized store, not the join slot.
    assert_own_return(&stderr, "pick_cond", "Owned");

    // match_return: borrowed enum-field arm vs empty owned arm -> Join.
    assert_own_return(&stderr, "deliver", "Join");

    // field-view family (fixed/clean): a field view and a whole-arg view are plain
    // parameter Borrows — never freed at the return.
    assert_own_return(&stderr, "getf", "Borrowed");
    assert_own_return(&stderr, "whole", "Borrowed");
}

/// Stage 1.5 (Gaps A+B): the analysis surfaces the FREE SITES the value
/// classification alone does not — the append element source-free
/// (`elem_accumulate`) and the return-buffer-aliasing delivery (`match_return`) —
/// each with the freed value's class and the borrow base to materialise from. Still
/// inert; this is the context the Stage-3 fix reads. See ownership-analysis-gaps.md.
#[test]
fn ownership_surfaces_free_sites() {
    let stderr = dump(OWN_SRC);

    // elem_accumulate: `out += [pick(t,i)]` source-frees pick's JOIN return. The
    // source is the inline `pick(…)` call; the unification oracle resolves its base
    // INTERPROCEDURALLY to the CALLER's argument `t` (pick's return borrows param `t`)
    // — exactly the witness the Stage-3 runtime guard needs.
    assert_free_site(&stderr, "collect", "AppendSource", "Join", "t");

    // match_return: the retbuf `_mv_items_1` is reassigned to a BORROWED enum-field
    // view (`OpGetField(e,…)`) → freeing the buffer over-frees `e`'s field. The
    // borrow base `e` is carried so the materialise copies the field into the buffer.
    assert_free_site(&stderr, "deliver", "ParamDeliver", "Borrowed", "e");

    // the clean shapes carry NO over-free site: a returned param borrow is not freed.
    assert_no_free_site(&stderr, "getf");
    assert_no_free_site(&stderr, "whole");
    // local_source's bug is the reassign, not an append/deliver: its pool-build
    // append lowers WITHOUT the 0x8000 source-free bit, so no AppendSource site.
    assert_no_free_site(&stderr, "pick_cond");
}

/// Assert the FULL ownership of a function's return, base included
/// (`OWN fn=n_<func> return=<Owned|Borrowed(base=…)|Join(base=…)>`).
fn assert_return_own(stderr: &str, func: &str, want: &str) {
    let line = stderr
        .lines()
        .find(|l| l.starts_with(&format!("OWN fn=n_{func} return=")))
        .unwrap_or_else(|| panic!("no OWN return line for {func}; dump:\n{stderr}"));
    assert!(
        line.ends_with(&format!("return={want}")),
        "{func} return: expected {want}, got: {line}"
    );
}

/// The unification oracle's BORROW BASE — the witness the Stage-3 runtime guard
/// needs — compared against hand-computed ground truth across the shapes, including
/// the INTERPROCEDURAL call→arg translation (the new piece) and the documented
/// retbuf-delivery approximation. This is the fact every own-vs-borrow site will
/// read; the test validates it in isolation before any chokepoint consumes it.
#[test]
fn ownership_resolves_the_borrow_base() {
    let stderr = dump(OWN_SRC);

    // direct projection / `??` join roots its base to the borrowed PARAM.
    assert_return_own(&stderr, "pick", "Join(base=t)"); // t[i] ?? dflt()
    assert_return_own(&stderr, "deliver", "Join(base=e)"); // match arm of e
    assert_return_own(&stderr, "nested", "Borrowed(base=o)"); // o.inner.rows — chain → o

    // INTERPROCEDURAL: `out += [pick(t,i)]`'s source is the `pick` CALL; the oracle
    // maps pick's borrowed param `t` to collect's argument `t`. (Also asserted as a
    // free site above — pinned here as the return/base contract.)
    assert_free_site(&stderr, "collect", "AppendSource", "Join", "t");

    // the displaced-owned reassign's borrow arm roots to the local `pool`.
    assert_reassign(&stderr, "pick_cond", "chosen", "Owned", "Join"); // rhs base = pool
    let cond_line = stderr
        .lines()
        .find(|l| l.contains("fn=n_pick_cond reassign") && l.contains("(chosen)"))
        .unwrap();
    assert!(
        cond_line.ends_with("rhs=Join(base=pool)"),
        "pick_cond chosen rhs base: {cond_line}"
    );

    // KNOWN APPROXIMATION (retbuf delivery): a whole-field / whole-arg return is
    // delivered through `__retbuf`, whose fill is not a tracked Set — so the base
    // resolves to `__retbuf`, not the true source `b`/`v`. Harmless: these are clean
    // field-return sites, never an over-free. Pinned so a future fix is a visible diff.
    assert_return_own(&stderr, "getf", "Borrowed(base=__retbuf)");
    assert_return_own(&stderr, "whole", "Borrowed(base=__retbuf)");
}

// ── @PLN85 match_return — the resisting case, pinned against the oracle ─────────
// The match_return codegen collapse is still open (the retbuf-promotion site). These
// tests pin what the NEW routine (`ownership_of`) classifies for the resisting
// variants, so the fix has a VERIFIED SPEC and the precise discriminator is locked.
// Key finding: the RETURN verdict `Join` is NOT the discriminator — it over-classifies
// the fresh-build case (`deliver3 → Join(base=o)`, the retbuf-param approximation,
// runtime-clean). The precise fix signal is the `ParamDeliver` FREE SITE: a retbuf
// reassigned to a borrowed enum-field view (base = an EXTERNAL var `e`), which the
// genuinely-leaking arms have and the fresh-build does not.
const MATCH_VARIANTS_SRC: &str = r#"
struct E { hp: integer not null, name: text }
fn e_default() -> E { E{hp:0, name:""} }
// V1: Filled arm borrows `e`; else arm is owned `[]` — a runtime JOIN (LEAKS today).
enum Cell { Empty, Filled { items: vector<E> } }
fn deliver(e: Cell) -> vector<E> { match e { Filled { items } => { items }, _ => { [] } } }
// V2: BOTH field arms borrow `e` (+ implicit owned default) — also a borrow-of-`e`.
enum Two { A { xs: vector<E> }, B { ys: vector<E> } }
fn deliver2(e: Two) -> vector<E> { match e { A { xs } => { xs }, B { ys } => { ys } } }
// V3: the Filled arm builds a FRESH owned vector `o` — owned, runtime-clean. The
// oracle over-classifies its RETURN as Join(base=o) (the retbuf approximation), but
// it has NO ParamDeliver site (the precise discriminator excludes it).
fn deliver3(e: Cell) -> vector<E> {
  match e { Filled { items } => { o: vector<E> = []; for x in 0..len(items) { o += [items[x] ?? e_default()]; } o }, _ => { [] } }
}
fn main() {
  c = Filled { items: [] }; r = deliver(c);
  t = A { xs: [] }; r2 = deliver2(t);
  r3 = deliver3(c);
  print("{len(r)} {len(r2)} {len(r3)}\n");
}
"#;

/// The match_return resisting cases pinned against the oracle. The `ParamDeliver`
/// FREE SITE — a retbuf aliased to a borrowed enum-field view whose base is an
/// EXTERNAL var — is the precise fix discriminator; the `Join` RETURN verdict alone
/// is not (it over-classifies the fresh-build `deliver3`). This is the verified spec
/// the still-open codegen fix must act on: materialise exactly the ParamDeliver arms.
#[test]
fn ownership_pins_match_return_resisting_cases() {
    let stderr = dump(MATCH_VARIANTS_SRC);

    // V1/V2 — the GENUINE over-free arms: the retbuf is aliased to a borrowed
    // enum-field view of the EXTERNAL subject `e`. The ParamDeliver site (base=e) is
    // the precise fix signal; the return verdict is Join(base=e).
    assert_free_site(&stderr, "deliver", "ParamDeliver", "Borrowed", "e");
    assert_return_own(&stderr, "deliver", "Join(base=e)");
    assert_free_site(&stderr, "deliver2", "ParamDeliver", "Borrowed", "e");
    assert_return_own(&stderr, "deliver2", "Join(base=e)");

    // V3 — the FRESH-BUILD arm: owned, runtime-clean. The oracle OVER-classifies its
    // RETURN as Join(base=o) (the retbuf-param approximation — `o` is the owned retbuf
    // classified as borrowed-of-itself), but there is NO ParamDeliver site. So the fix,
    // keyed on ParamDeliver (NOT the return verdict), correctly LEAVES deliver3 ALONE.
    assert_return_own(&stderr, "deliver3", "Join(base=o)"); // documented over-classification
    assert_no_free_site(&stderr, "deliver3"); // the precise discriminator excludes it
}

// ── @PLN85 Stage-3 site 1: the `local_source` compiler wiring (LOFT_JOIN_OWN) ───
// The displaced-owned-store leak: `chosen = dflt()` move-adopts a fresh store into
// `chosen` (the source retbuf the cleanup guards is left null), then `chosen =
// pool[wj]` orphans it. The fix strips `chosen`'s flattened `["pool"]` dep so it is
// OWNED everywhere — the owned path deep-copies the borrow into `chosen`'s store and
// frees it. VALUE-correct either way (the leak does not corrupt); only the leak
// differs, so the gate is the `not freed` warning. See ownership-analysis-gaps.md.

const LOCAL_SOURCE_SRC: &str = r#"
struct M { hp: integer not null, name: text }
fn dflt() -> M { M{hp:-1, name:"d"} }
fn pick(t: vector<M>, salt: integer) -> M {
  pool: vector<M> = []; for p in 0..len(t) { pool += [t[p] ?? dflt()]; }
  chosen = dflt();
  np = len(pool);
  for wj in 0..np { if salt % np == wj { chosen = pool[wj] ?? dflt(); } }
  chosen
}
fn main() {
  t: vector<M> = []; for k in 0..4 { t += [M{hp:k * 10, name:"m"}]; }
  // np=4: pick(t, salt) selects pool[salt % 4] -> hp = (salt % 4) * 10.
  for i in 0..8 {
    r = pick(t, i);
    assert(r.hp == (i % 4) * 10, "pick i{i} hp={r.hp} want={(i % 4) * 10}");
    assert(len(t) == 4, "src corrupted i{i} len={len(t)}");
  }
  print("local-source ok\n");
}
"#;

/// Run a source on a backend, optionally with `LOFT_JOIN_OWN`; return (stdout, stderr).
fn run_backend(src: &str, backend: &str, join_own: bool) -> (String, String) {
    use std::hash::{Hash, Hasher};
    let dir = std::env::temp_dir().join("loft_join_own");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    backend.hash(&mut h);
    join_own.hash(&mut h);
    let path = dir.join(format!("probe_{:016x}.loft", h.finish()));
    std::fs::write(&path, src).expect("write probe");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.args([backend])
        .arg(&path)
        .env("LOFT_STORES", "warn")
        .env("LOFT_NATIVE_LEAK_CHECK", "1")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "180");
    if join_own {
        // Post-flip the fixes are DEFAULT-ON: the on-leg actively removes the
        // opt-out so an ambient `LOFT_NO_JOIN_OWN` cannot invert the premise.
        cmd.env_remove("LOFT_NO_JOIN_OWN");
    } else {
        // The control leg (the documented pre-fix behaviour) opts out.
        cmd.env("LOFT_NO_JOIN_OWN", "1");
    }
    let out = cmd.output().expect("spawn loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The fix: under `LOFT_JOIN_OWN` the displaced-owned `local_source` shape is
/// VALUE-correct AND leak-free on BOTH backends.
#[test]
fn join_own_fixes_local_source_both_backends() {
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run_backend(LOCAL_SOURCE_SRC, backend, true);
        assert!(
            stdout.contains("local-source ok"),
            "{backend} value-incorrect under LOFT_JOIN_OWN:\nstdout:{stdout}\nstderr:{stderr}"
        );
        assert!(
            !stderr.contains("not freed"),
            "{backend} still leaks under LOFT_JOIN_OWN:\n{stderr}"
        );
    }
}

// ── @PLN85 unification (first chokepoint collapse): elem_accumulate, interp ────
// `out += [pick(t,i)]` source-frees pick's `t[i] ?? m_none()` JOIN return. The
// interp first-bind now reads the ownership oracle: OpBindOrCopy adopts the owned
// `m_none()` arm and materialises the borrowed `t[i]` arm, witnessed by the oracle's
// interprocedurally-resolved base `t`. BOTH arms must be value-correct + clean.
const ELEM_SRC: &str = r#"
struct M { hp: integer not null, name: text }
fn m_none() -> M { M{hp:-1, name:"n"} }
fn pick(t: vector<M>, i: integer) -> M { t[i] ?? m_none() }
fn collect(t: vector<M>) -> vector<M> { out: vector<M> = []; for i in 0..len(t) { out += [pick(t, i)]; } out }
fn collect_owned(t: vector<M>) -> vector<M> { out: vector<M> = []; for i in 0..3 { out += [pick(t, i + 100)]; } out }
fn filler(n: integer) -> integer { es: vector<M> = []; for j in 0..n { es += [M{hp:j, name:"f"}]; } return len(es); }
fn main() {
  t: vector<M> = []; for k in 0..3 { t += [M{hp:k * 10, name:"m"}]; }
  for i in 0..6 {
    r = collect(t); acc = 0; for f in 0..6 { acc += filler(6); }
    assert(len(t) == 3, "src corrupted i{i}={len(t)}");
    assert(len(r) == 3, "borrow-arm len i{i}={len(r)}");
    o = collect_owned(t);
    assert(len(o) == 3, "owned-arm len i{i}={len(o)}");
  }
  print("elem-accumulate ok\n");
}
"#;

/// The unification collapse fixes BOTH arms of elem_accumulate on BOTH backends (the
/// borrow arm's interp UAF and the owned arm's leak — native's `_src == _dst` guard
/// leaked the owned arm too) — value-correct and clean under LOFT_JOIN_OWN, witnessed
/// by the oracle's interprocedurally-resolved base.
#[test]
fn join_own_fixes_elem_accumulate_both_backends() {
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run_backend(ELEM_SRC, backend, true);
        assert!(
            stdout.contains("elem-accumulate ok"),
            "{backend} value-incorrect under LOFT_JOIN_OWN:\nstdout:{stdout}\nstderr:{stderr}"
        );
        assert!(
            !stderr.contains("not freed"),
            "{backend} still leaks under LOFT_JOIN_OWN:\n{stderr}"
        );
    }
}

/// The GATE: WITHOUT the flag the same shape is value-correct but LEAKS the
/// displaced owned store — pins that the flag is what closes the leak (so a future
/// regression that silently stops stripping is caught), on both backends.
#[test]
fn local_source_leaks_without_join_own() {
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run_backend(LOCAL_SOURCE_SRC, backend, false);
        assert!(stdout.contains("local-source ok"), "{backend}: {stderr}");
        assert!(
            stderr.contains("not freed"),
            "{backend}: expected the displaced-owned LEAK without the flag, got none:\n{stderr}"
        );
    }
}

// ── @PLN85 match_return — the gated owned-copy synthesis (P1) ───────────────────
// `jo_copy_borrowed_arm_yield` (src/parser/control.rs) rewrites a borrowed enum-field
// arm yield (`Filled { items } => { items }`) into an owned copy
// (`{ o = []; o += items; o }`), so the return escapes OWNED rather than as a view
// into the match subject. Two plain-loft codegen bugs blocked it — the gen_if arm-join
// discard and the empty value-block push (tests/scripts/441 + 442) — and are now fixed,
// so the whole-append owned copy "just works" (no element-loop synthesis needed).

const MATCH_RETURN_SRC: &str = r#"
struct E { hp: integer not null, name: text }
enum Cell { Empty, Filled { items: vector<E> } }
fn deliver(e: Cell) -> vector<E> { match e { Filled { items } => { items }, _ => { [] } } }
fn filler(n: integer) -> integer { es: vector<E> = []; for j in 0..n { es += [E{hp:j, name:"f"}]; } return len(es); }
fn main() {
  inner: vector<E> = []; for k in 0..3 { inner += [E{hp:k * 10, name:"m"}]; }
  cell = Filled { items: inner };
  for i in 0..6 {
    r = deliver(cell); acc = 0; for f in 0..6 { acc += filler(6); }
    assert(len(inner) == 3, "src corrupted i{i}={len(inner)}");
    assert(len(r) == 3, "match-return len i{i}={len(r)}");
    assert(r[0].hp == 0 && r[2].hp == 20, "match-return values i{i}");
    e = deliver(Empty {});
    assert(len(e) == 0, "empty-arm i{i}={len(e)}");
  }
  print("match-return ok\n");
}
"#;

/// Run `loft introspect` on a source, optionally with `LOFT_JOIN_OWN`; return stdout
/// (the IR dump, used to read the emitted return-type dependency).
fn introspect(src: &str, join_own: bool) -> String {
    use std::hash::{Hash, Hasher};
    let dir = std::env::temp_dir().join("loft_join_own");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    "introspect".hash(&mut h);
    join_own.hash(&mut h);
    let path = dir.join(format!("introspect_{:016x}.loft", h.finish()));
    std::fs::write(&path, src).expect("write probe");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("introspect").arg(&path).env("LOFT_NO_CACHE", "1");
    if join_own {
        cmd.env_remove("LOFT_NO_JOIN_OWN");
    } else {
        cmd.env("LOFT_NO_JOIN_OWN", "1");
    }
    let out = cmd.output().expect("spawn loft introspect");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The `deliver` function's IR signature line from an introspect dump (carries the
/// top-block dependency set, e.g. `…["__retbuf", "e"]`).
fn deliver_signature(dump: &str) -> String {
    dump.lines()
        .find(|l| l.starts_with("fn n_deliver(e:ref"))
        .unwrap_or_else(|| panic!("no deliver signature in introspect dump:\n{dump}"))
        .to_string()
}

/// The gated synthesis is value-correct AND leak-free on BOTH backends now that the
/// blocking codegen bugs are fixed — the whole-append owned copy "just works".
#[test]
fn join_own_match_return_synthesis_both_backends() {
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run_backend(MATCH_RETURN_SRC, backend, true);
        assert!(
            stdout.contains("match-return ok"),
            "{backend} value-incorrect under LOFT_JOIN_OWN:\nstdout:{stdout}\nstderr:{stderr}"
        );
        assert!(
            !stderr.contains("not freed"),
            "{backend} leaks under LOFT_JOIN_OWN:\n{stderr}"
        );
    }
}

/// The GATE discriminator. The borrowed-arm return BORROWS the match subject `e`
/// without the flag (`["__retbuf", "e"]`) and is OWNED with it (`["__retbuf"]`). The
/// runtime is clean either way now (the over-free no longer surfaces as a leak once
/// the codegen bugs are fixed), so the emitted return dependency is the flag's
/// observable effect — this pins that the synthesis is what strips the borrow.
#[test]
fn join_own_match_return_strips_the_borrow() {
    let off = deliver_signature(&introspect(MATCH_RETURN_SRC, false));
    let on = deliver_signature(&introspect(MATCH_RETURN_SRC, true));
    assert!(
        off.contains("[\"__retbuf\", \"e\"]"),
        "without the flag deliver should borrow `e`, got: {off}"
    );
    assert!(
        on.contains("[\"__retbuf\"]"),
        "with the flag deliver should be owned (no `e` borrow), got: {on}"
    );
}

// ── @PLN90 phase 1 — construction / field-append copies are covered by the verdict ──
// A struct/enum field built from an existing vector deep-copies it (the field owns its
// data). Before @PLN90 the verdict saw only the var-buffer copy idiom (`o = src` / `o +=
// src`) and missed this dominant category entirely; now it emits a Copy row so the
// copy-vs-borrow decision covers it (diagnostic only — always Copy, never an ElidePlan).

const CONSTRUCT_SRC: &str = r#"
struct E { hp: integer not null, name: text }
struct Box { rows: vector<E> }
fn make_box(s: vector<E>) -> Box { Box { rows: s } }
fn main() {
  s: vector<E> = []; for k in 0..3 { s += [E{hp:k, name:"s"}]; }
  b = make_box(s);
  assert(len(b.rows) == 3, "rows");
  print("construct ok\n");
}
"#;

#[test]
fn construction_copy_is_covered_by_the_verdict() {
    let stderr = dump(CONSTRUCT_SRC);
    // `Box { rows: s }` deep-copies `s` into the new record's field — a Copy the verdict
    // now classifies (it formerly recorded only var-target appends).
    assert_verdict(&stderr, "make_box", "Copy");
    assert!(
        stderr.contains("construction/field-append copy"),
        "the construction copy should carry the construction reason; dump:\n{stderr}"
    );
    // @PLN90 phase 2 — a construction copy is IMPLICIT to the model (the field owns its
    // data) → silent, NOT warned. It is not the avoidable worklist.
    assert!(
        stderr
            .lines()
            .any(|l| l.contains("fn=n_make_box ") && l.contains("bucket=implicit")),
        "construction copy should be bucket=implicit (silent); dump:\n{stderr}"
    );
}

// ── @PLN90 phase 1 — the return-buffer (field / whole-vector return) copy is covered ──
// `fn f(b: Box) -> vector { b.rows }` materialises `b.rows` into the passed-in return
// buffer (`__retbuf`). The buffer is an argument, not a fresh `OpDatabase` local, so the
// var-buffer copy idiom skipped it; now a Copy row covers it (diagnostic only — eliding it
// to a borrow would be the P4 borrowed-return).
const FIELD_RETURN_SRC: &str = r#"
struct E { hp: integer not null, name: text }
struct Box { rows: vector<E> }
fn field_ret(b: Box) -> vector<E> { b.rows }
fn main() {
  s: vector<E> = []; for k in 0..3 { s += [E{hp:k, name:"s"}]; }
  bx = Box { rows: s };
  r = field_ret(bx);
  assert(len(r) == 3, "rows");
  print("field-return ok\n");
}
"#;

#[test]
fn field_return_copy_is_covered_by_the_verdict() {
    let stderr = dump(FIELD_RETURN_SRC);
    assert_verdict(&stderr, "field_ret", "Copy");
    assert!(
        stderr.contains("materialised into the return buffer"),
        "the field-return copy should carry the return-buffer reason; dump:\n{stderr}"
    );
    // @PLN90 phase 2 — a field-return copy is AVOIDABLE (bucket 2, the elimination
    // worklist): a borrowed-view return is sound; the copy is only there because the
    // borrowed return path is not yet correct (@PLN85 P4).
    assert!(
        stderr
            .lines()
            .any(|l| l.contains("fn=n_field_ret ") && l.contains("bucket=AVOIDABLE")),
        "field-return copy should be bucket=AVOIDABLE; dump:\n{stderr}"
    );
}

// ── @PLN90 phase 1 — the `OpCopyRecord` record copy is covered (the last gap) ──
// `v[i] = e` deep-copies the record `e` into the element slot. This is not append-based,
// so neither the var-buffer idiom nor the construction / return-buffer branches see it;
// now a Copy row covers it (diagnostic only). The same-var no-op alias is excluded.
const RECORD_COPY_SRC: &str = r#"
struct E { hp: integer not null, name: text }
fn set_one(v: vector<E>, e: E) -> integer { v[1] = e; return v[1].hp; }
fn main() {
  v: vector<E> = []; for k in 0..3 { v += [E{hp:k, name:"v"}]; }
  e = E{hp:99, name:"z"};
  r = set_one(v, e);
  assert(r == 99, "set");
  print("record-copy ok\n");
}
"#;

#[test]
fn record_copy_is_covered_by_the_verdict() {
    let stderr = dump(RECORD_COPY_SRC);
    assert!(
        stderr
            .lines()
            .any(|l| l.contains("fn=n_set_one ") && l.contains("record deep-copy (OpCopyRecord)")),
        "the `v[i] = e` record copy should be classified Copy [record deep-copy]; dump:\n{stderr}"
    );
}

// ── @PLN90 phase A — the bound-vs-unbound survival split + the Internal (item 1) bucket ──
// Under `LOFT_COPY_SURVIVAL` a construction / record copy is classified by its SOURCE FATE:
// a MOVE (source consumed) is silent (Implicit); a still-live source duplicated is indicated
// (Avoidable read-only / Forced mutated); a copy whose SOURCE is a compiler-generated temp
// (`_`-prefixed) is `Internal` — a developer-worklist copy excluded from the user report.
const SURVIVAL_SRC: &str = r#"
struct Wrap { data: vector<integer> }
fn mk() -> Wrap { Wrap { data: [1, 2, 3] } }
fn cmove() { inner: vector<integer> = [1,2,3]; s = Wrap { data: inner }; print("{s.data[0]}\n"); }
fn csurv() { inner: vector<integer> = [1,2,3]; s = Wrap { data: inner }; print("{inner[0]} {s.data[0]}\n"); }
fn cmut()  { inner: vector<integer> = [1,2,3]; s = Wrap { data: inner }; inner += [4]; print("{s.data[0]} {inner[3]}\n"); }
fn internal_src(v: vector<Wrap>) -> vector<Wrap> { r = v; r += [mk()]; r }
fn main() { cmove(); csurv(); cmut(); a = internal_src([]); print("{len(a)}\n"); }
"#;

/// Like `dump`, but with the survival split flag on.
fn dump_survival(src: &str) -> String {
    use std::hash::{Hash, Hasher};
    let dir = std::env::temp_dir().join("loft_use_analysis");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    "survival".hash(&mut h);
    let path = dir.join(format!("probe_{:016x}.loft", h.finish()));
    std::fs::write(&path, src).expect("write probe");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret", "--check"])
        .arg(&path)
        .env("LOFT_MATERIALIZE_DUMP", "1")
        .env("LOFT_COPY_SURVIVAL", "1")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NO_JOIN_OWN", "1")
        .output()
        .expect("spawn loft");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn assert_bucket(stderr: &str, func: &str, bucket: &str) {
    assert!(
        stderr.lines().any(
            |l| l.contains(&format!("fn=n_{func} ")) && l.contains(&format!("bucket={bucket}"))
        ),
        "{func}: expected a copy row bucket={bucket}; dump:\n{stderr}"
    );
}

#[test]
fn survival_split_bound_vs_unbound_and_internal() {
    let stderr = dump_survival(SURVIVAL_SRC);
    // Bound → silent; unbound → indicated; compiler-generated source → Internal.
    assert_bucket(&stderr, "cmove", "implicit"); // move: source consumed
    assert_bucket(&stderr, "csurv", "AVOIDABLE"); // unbound, read-only survivor
    assert_bucket(&stderr, "cmut", "forced"); // unbound, mutated after
    assert_bucket(&stderr, "internal_src", "internal"); // `_`-prefixed (mk() result) source

    // Item 1: the Internal row must NOT leak into the user-facing worklist tally — the
    // internal copy is counted separately, never as avoidable/forced for the user.
    assert!(
        stderr
            .lines()
            .any(|l| l.contains("MAT-WORKLIST") && l.contains("internal_copies=")),
        "the worklist tally must carry an internal_copies count; dump:\n{stderr}"
    );

    // Default (flag OFF) keeps every construction copy Implicit (byte-identical) — the split
    // is gated, so with it off cmove/csurv/cmut are all silent.
    let off = dump(SURVIVAL_SRC);
    for f in ["cmove", "csurv", "cmut"] {
        assert_bucket(&off, f, "implicit");
    }
}
