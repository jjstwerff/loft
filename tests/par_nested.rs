// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN117 — `par` inside a function that is itself a `par` worker.
//!
//! The interpreter used to abort the whole run here (*"parallel_queue called
//! outside State::execute()"*) while `--native` computed the right answer, so
//! the same program meant two different things depending on the backend — and
//! in the browser, which runs the interpreter, it meant a dead page.  A worker
//! now inherits its parent's program context (`ParallelCtx`), so a nested
//! dispatch has the bytecode, library and `Data` it needs.
//!
//! Every cell asserts a **hand-computed** value rather than only comparing the
//! two backends: two implementations agreeing is not evidence that either is
//! right.  `cross_mode!` then adds the cross-backend check on top.
//!
//! ```bash
//! cargo test --release --test par_nested
//! ```

mod common;

// ── Depth 2 — a par worker that itself runs a par ────────────────────────────
//
// mid(x) = x + (1+…+8) = x + 36, and main sums mid over 1..=16:
// (1+…+16) + 16×36 = 136 + 576 = 712.
cross_mode!(
    nested_depth_two,
    r#"
    fn leaf(r: integer) -> integer { return r; }

    fn mid(x: integer) -> integer {
        rows = [1, 2, 3, 4, 5, 6, 7, 8];
        acc = x;
        for r in rows par(v = leaf(r), 4) { acc += v; }
        return acc;
    }

    fn test() {
        data = [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16];
        sum = 0;
        for a in data par(b = mid(a), 4) { sum += b; }
        assert(sum == 712, "nested depth 2 sum");
        print("nested_depth_two {sum}\n");
    }
    "#
);

// ── Depth 3 — nesting is not special-cased at one level ──────────────────────
//
// lvl2(y) = y + (1+2+3+4) = y + 10; lvl1(x) = x + Σ(r+10 for r in 1..=8)
// = x + 36 + 80 = x + 116; main = (1+…+16) + 16×116 = 136 + 1856 = 1992.
cross_mode!(
    nested_depth_three,
    r#"
    fn leaf(r: integer) -> integer { return r; }

    fn lvl2(y: integer) -> integer {
        rows2 = [1, 2, 3, 4];
        acc2 = y;
        for r2 in rows2 par(v2 = leaf(r2), 2) { acc2 += v2; }
        return acc2;
    }

    fn lvl1(x: integer) -> integer {
        rows1 = [1, 2, 3, 4, 5, 6, 7, 8];
        acc1 = x;
        for r1 in rows1 par(v1 = lvl2(r1), 4) { acc1 += v1; }
        return acc1;
    }

    fn test() {
        data = [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16];
        sum = 0;
        for a in data par(b = lvl1(a), 4) { sum += b; }
        assert(sum == 1992, "nested depth 3 sum");
        print("nested_depth_three {sum}\n");
    }
    "#
);

// ── A different return shape — text, so a second dispatcher is covered ───────
//
// The text stitch is a separate entry point (it failed as
// "parallel_queue_text called outside State::execute()"), so it is worth its
// own cell: tmid(1)="1123", tmid(2)="2123", concatenated in row order.
cross_mode!(
    nested_text_return,
    r#"
    fn tleaf(r: integer) -> text { return "{r}"; }

    fn tmid(x: integer) -> text {
        rows = [1, 2, 3];
        out = "{x}";
        for r in rows par(v = tleaf(r), 2) { out += v; }
        return out;
    }

    fn test() {
        data = [1, 2];
        s = "";
        for a in data par(b = tmid(a), 2) { s += b; }
        assert(s == "11232123", "nested text concat");
        print("nested_text_return {s}\n");
    }
    "#
);
