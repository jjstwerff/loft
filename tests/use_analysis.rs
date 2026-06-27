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

fn main() {
  s = Sim { tiles: [1, 2, 3], walls: [0, 0, 0] };
  print("{tile_at(s, 1)} {edge_at(s, 1)} {mutate(s, 1)} {local_src(1)} {to_user(s)}\n");
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
}
