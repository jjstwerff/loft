// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN16 M5d phase 2 — the `--rpc` debug server, driven over an in-memory pipe.
//!
//! Sends NDJSON requests through `rpc::run_rpc` and asserts the NDJSON responses +
//! events: launch → setBreakpoints → run (→ `stopped` in the right frame) → eval
//! (JSON value) → continue (→ program `output` + `terminated`).  This is the surface
//! an agent / CI drives the debugger through.

use std::io::Cursor;

/// A unique temp path for a `.loft` program, keyed by tag + pid.
fn tmp_program(tag: &str, src: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("loft_rpc_{tag}_{}.loft", std::process::id()));
    std::fs::write(&p, src).expect("write temp program");
    p
}

/// Run the RPC server over `requests` (joined by newlines) and return the output text.
fn drive(requests: &[String]) -> String {
    let input = Cursor::new(requests.join("\n").into_bytes());
    let mut out: Vec<u8> = Vec::new();
    loft::rpc::run_rpc("default", input, &mut out).expect("rpc run");
    String::from_utf8(out).expect("utf8")
}

#[test]
fn rpc_launch_break_eval_continue() {
    // helper's body is line 2; main calls it then prints.
    let path = tmp_program(
        "basic",
        "fn helper(n: integer) -> integer {\n  n * 2\n}\nfn main() {\n  a = helper(21);\n  print(\"a={a}\")\n}\n",
    );
    let file = path.to_str().unwrap();
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":2}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"eval\",\"expr\":\"n\"}".to_string(),
        "{\"id\":5,\"req\":\"continue\"}".to_string(),
        "{\"id\":6,\"req\":\"disconnect\"}".to_string(),
    ]);

    // Every output line is a JSON object.
    for line in out.lines() {
        assert!(
            line.starts_with('{') && line.ends_with('}'),
            "not JSON: {line}"
        );
    }
    assert!(out.contains("\"id\":1,\"ok\":true"), "launch ok: {out}");
    // run → stopped at the breakpoint, inside `helper`.
    assert!(out.contains("\"event\":\"stopped\""), "a stop event: {out}");
    assert!(
        out.contains("\"function\":\"helper\""),
        "stopped in helper: {out}"
    );
    // eval `n` against the frame → the JSON value 21 (helper's argument).
    assert!(
        out.contains("\"id\":4,\"ok\":true,\"value\":21"),
        "eval n == 21: {out}"
    );
    // continue → the program prints, captured as an `output` event, then terminates.
    assert!(
        out.contains("\"category\":\"stdout\",\"text\":\"a=42\""),
        "program output captured: {out}"
    );
    assert!(
        out.contains("\"event\":\"terminated\""),
        "terminated: {out}"
    );

    let _ = std::fs::remove_file(&path);
}

// A conditional breakpoint whose condition reads a struct field: break only on the
// matching call, then eval a scalar field.
#[test]
fn rpc_conditional_breakpoint_struct_field() {
    let path = tmp_program(
        "proba",
        "struct Point { x: integer, y: integer }\n\
         fn use_pt(p: Point) -> integer {\n  p.x + p.y\n}\n\
         fn main() {\n  use_pt(Point { x: 1, y: 2 });\n  use_pt(Point { x: 9, y: 2 })\n}\n",
    );
    let file = path.to_str().unwrap();
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":3,\"condition\":\"p.x == 9\"}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"eval\",\"expr\":\"p.x\"}".to_string(),
        "{\"id\":5,\"req\":\"continue\"}".to_string(),
        "{\"id\":6,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(
        out.contains("\"event\":\"stopped\""),
        "stopped on the matching call: {out}"
    );
    assert!(
        out.contains("\"id\":4,\"ok\":true,\"value\":9"),
        "eval p.x == 9: {out}"
    );
    assert!(
        out.contains("\"event\":\"terminated\""),
        "terminated: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// Plain (unconditional) break, then eval the whole struct — returned as a JSON object
// via loft's inbuilt `.to_json()`.
#[test]
fn rpc_eval_struct_as_json() {
    let path = tmp_program(
        "probb",
        "struct Point { x: integer, y: integer }\n\
         fn use_pt(p: Point) -> integer {\n  p.x + p.y\n}\n\
         fn main() {\n  use_pt(Point { x: 9, y: 2 })\n}\n",
    );
    let file = path.to_str().unwrap();
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":3}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"eval\",\"expr\":\"p\"}".to_string(),
        "{\"id\":5,\"req\":\"continue\"}".to_string(),
        "{\"id\":6,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(out.contains("\"event\":\"stopped\""), "stopped: {out}");
    // Eval of a bare struct → JSON object via loft's inbuilt `.to_json()`.
    assert!(
        out.contains("\"value\":{\"x\":9,\"y\":2}"),
        "eval p as JSON: {out}"
    );
    assert!(
        out.contains("\"event\":\"terminated\""),
        "terminated: {out}"
    );
    let _ = std::fs::remove_file(&path);
}
