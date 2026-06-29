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
    let dir = std::env::temp_dir().join("loft_use_analysis");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let path = dir.join("probe.loft");
    let mut f = std::fs::File::create(&path).expect("write probe");
    f.write_all(src.as_bytes()).expect("write probe body");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret", "--check"])
        .arg(&path)
        .env("LOFT_MATERIALIZE_DUMP", "1")
        .env("LOFT_NO_CACHE", "1")
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
        .env("LOFT_NO_CACHE", "1");
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
