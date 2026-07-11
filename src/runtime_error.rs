// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I75 — Diagnostics collector

//! Plan-07 phase 4 — typed runtime errors.
//!
//! Replaces the implicit "panic = bug; sentinel = user error" coin-flip
//! with an explicit [`RuntimeError`] raised at a small set of well-known
//! fault sites.  After phase 4, every fault attributable to user code is
//! a `RuntimeError` with a source position and a stable kind; anything
//! NOT attributable to user code stays a hard panic and is treated as
//! an interpreter bug.
//!
//! Lives on [`crate::database::Stores`] (`runtime_error: Option<Box<RuntimeError>>`)
//! so native fns — which only see `&mut Stores`, not `&mut State` — can
//! populate it.  The interpreter's dispatch loop in
//! `src/state/mod.rs::execute_argv` checks
//! `database.runtime_error.is_some()` after each op and breaks the loop
//! by setting `code_pos = u32::MAX`.  `main.rs` then renders the error
//! through the phase-2 pretty renderer.
//!
//! See `doc/claude/plans/07-error-messages/04-runtime-error-kinds.md`
//! for the per-site conversion list and the rationale for raising vs
//! returning a sentinel (sentinel for `??` lhs; raise otherwise).

// Phase 4 ships infrastructure + the first two fault sites
// (UserPanic, AssertionFailed); the remaining variants and the
// `op_pc` / `kind` field are exercised once steps 4.3-4.10 land.
// Suppress dead-code warnings module-wide until then to keep the
// `bin` target's clippy gate green without splattering per-variant
// allows.
#![allow(dead_code)]

use crate::diagnostics::{DiagEntry, Level};
use crate::lexer::Position;

/// A typed runtime fault attributable to user code.
///
/// Constructed by a fault-site opcode or native fn via the
/// `RuntimeError::*` constructors, stored on `Stores::runtime_error`,
/// surfaced by the dispatch loop, and rendered via
/// [`RuntimeError::to_diag_entry`] through the phase-2 pretty renderer.
#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub kind: RuntimeErrorKind,
    /// Source position of the offending construct, when known.  Resolved
    /// at raise time via `State::source_loc_for(op_pc)` for opcode
    /// faults, or passed directly by the loft surface call (e.g.
    /// `panic("msg")` injects file/line via stdlib stubs).
    pub position: Option<Position>,
    /// Bytecode `pc` of the dispatching op when the error was raised.
    /// `u32::MAX` when not raised from a bytecode op (e.g. native-only
    /// path, future native-runtime conversions).
    pub op_pc: u32,
    /// Free-form human-readable detail.  Kind-specific structured fields
    /// live on `kind`; this string is the rendered presentation tail
    /// shown after the kind label (e.g. `"divide by zero in `attack /
    /// armour`"`).
    pub message: String,
    /// Plan-07 phase 4g.1 / 4g.2 — call-chain at raise time
    /// (innermost first).  Each entry is the function's name as it
    /// appeared in source (the `n_<name>` registry strips its prefix).
    /// Empty when raised outside a function (e.g., top-level script
    /// scope) or when the State call_stack wasn't available (the
    /// Stores-side `raise_runtime` path leaves it empty — native
    /// codegen lacks call-stack capture today, slice-2 work).
    /// Rendered as `  in fn <innermost>() ← fn <next>() ← …` after
    /// the typed-error block in main.rs.
    pub call_chain: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum RuntimeErrorKind {
    /// Integer / float `/` or `%` with right-hand side `0`.
    DivideByZero,
    /// Vector / text positive-index access past the end.
    IndexOutOfBounds { idx: i64, len: u32 },
    /// Vector / text index < 0.
    NegativeIndex { idx: i64 },
    /// Field / method access through a null `DbRef`.
    NullDereference,
    /// Narrowing cast (e.g. `i64 -> i32`) overflowed the target range.
    NarrowCastOverflow { value: i64, target: &'static str },
    /// A `<<` / `>>` whose amount is outside `[0, 64)`, or whose result is the
    /// reserved `i64::MIN` null sentinel (@PLN102 null-model keystone,
    /// D-op-null-2): the shift cannot produce a representable non-null value, so
    /// it yields null and continues (like `÷0`), loudly rather than silently
    /// masking or nulling.
    ShiftOutOfRange,
    /// An `as` cast whose value cannot be represented in the target: a `float`
    /// outside the integer range, an integer that is not a valid Unicode code
    /// point, or a text that parses to exactly the reserved `i64::MIN` sentinel
    /// (@PLN102 D-op-null-2). Yields null and continues (like `÷0`), loudly
    /// rather than silently saturating or nulling a real value.
    CastOutOfRange,
    /// Recursion exceeded `State::MAX_CALL_DEPTH`.
    StackOverflow,
    /// `panic("msg")` builtin called from loft code.
    UserPanic { message: String },
    /// `assert(test, "msg", file, line)` builtin called with `test == false`.
    AssertionFailed { message: String },
}

impl RuntimeErrorKind {
    /// Stable, machine-readable kind label for log / test output.
    /// Mirrors the variant name verbatim in lower_snake_case.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            RuntimeErrorKind::DivideByZero => "divide_by_zero",
            RuntimeErrorKind::IndexOutOfBounds { .. } => "index_out_of_bounds",
            RuntimeErrorKind::NegativeIndex { .. } => "negative_index",
            RuntimeErrorKind::NullDereference => "null_dereference",
            RuntimeErrorKind::NarrowCastOverflow { .. } => "narrow_cast_overflow",
            RuntimeErrorKind::ShiftOutOfRange => "shift_out_of_range",
            RuntimeErrorKind::CastOutOfRange => "cast_out_of_range",
            RuntimeErrorKind::StackOverflow => "stack_overflow",
            RuntimeErrorKind::UserPanic { .. } => "user_panic",
            RuntimeErrorKind::AssertionFailed { .. } => "assertion_failed",
        }
    }

    /// Human-readable one-line description with structured-field detail
    /// inline.  Used as the `message` of the rendered `DiagEntry`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            RuntimeErrorKind::DivideByZero => "divide by zero".to_string(),
            RuntimeErrorKind::IndexOutOfBounds { idx, len } => {
                format!("index {idx} out of bounds for length {len}")
            }
            RuntimeErrorKind::NegativeIndex { idx } => {
                format!("negative index {idx}")
            }
            RuntimeErrorKind::NullDereference => "null dereference".to_string(),
            RuntimeErrorKind::NarrowCastOverflow { value, target } => {
                format!("value {value} overflows target type {target}")
            }
            RuntimeErrorKind::ShiftOutOfRange => {
                "shift amount out of range [0,64) or result is the reserved null value".to_string()
            }
            RuntimeErrorKind::CastOutOfRange => {
                "cast value cannot be represented in the target type".to_string()
            }
            RuntimeErrorKind::StackOverflow => "call stack overflow".to_string(),
            RuntimeErrorKind::UserPanic { message } => format!("panic: {message}"),
            RuntimeErrorKind::AssertionFailed { message } => {
                format!("assertion failed: {message}")
            }
        }
    }
}

impl RuntimeError {
    /// Construct a `UserPanic` error at the loft surface call site.
    /// `file` / `line` come from the `panic("msg", file, line)` stub
    /// arguments injected by the parser at the loft call site.
    #[must_use]
    pub fn user_panic(message: String, file: String, line: u32) -> Self {
        let position = if file.is_empty() {
            None
        } else {
            Some(Position { file, line, pos: 1 })
        };
        let kind = RuntimeErrorKind::UserPanic { message };
        let detail = kind.describe();
        Self {
            kind,
            position,
            op_pc: u32::MAX,
            message: detail,
            call_chain: Vec::new(),
        }
    }

    /// Construct an `AssertionFailed` error at the loft surface call site.
    #[must_use]
    pub fn assertion_failed(message: String, file: String, line: u32) -> Self {
        let position = if file.is_empty() {
            None
        } else {
            Some(Position { file, line, pos: 1 })
        };
        let kind = RuntimeErrorKind::AssertionFailed { message };
        let detail = kind.describe();
        Self {
            kind,
            position,
            op_pc: u32::MAX,
            message: detail,
            call_chain: Vec::new(),
        }
    }

    /// Convert to a `DiagEntry` so the existing phase-2 pretty renderer
    /// can format the error with `--> file:line:col` + source line +
    /// caret.  Errors without a position emit just the level + message.
    #[must_use]
    pub fn to_diag_entry(&self) -> DiagEntry {
        let (file, line, col) = self.position.as_ref().map_or_else(
            || (String::new(), 0, 0),
            |p| (p.file.clone(), p.line, p.pos),
        );
        DiagEntry {
            level: Level::Error,
            message: self.message.clone(),
            file,
            line,
            col,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_panic_carries_message_and_position() {
        let err = RuntimeError::user_panic("oops".into(), "test.loft".into(), 42);
        assert!(matches!(err.kind, RuntimeErrorKind::UserPanic { .. }));
        assert_eq!(err.kind.label(), "user_panic");
        assert!(err.message.contains("oops"));
        let pos = err.position.as_ref().expect("position present");
        assert_eq!(pos.file, "test.loft");
        assert_eq!(pos.line, 42);
    }

    #[test]
    fn assertion_failed_carries_message_and_position() {
        let err = RuntimeError::assertion_failed("x == 5".into(), "fixture.loft".into(), 7);
        assert!(matches!(err.kind, RuntimeErrorKind::AssertionFailed { .. }));
        assert_eq!(err.kind.label(), "assertion_failed");
        assert!(err.message.contains("x == 5"));
        let pos = err.position.as_ref().expect("position present");
        assert_eq!(pos.line, 7);
    }

    #[test]
    fn empty_file_means_no_position() {
        let err = RuntimeError::user_panic("oops".into(), String::new(), 0);
        assert!(err.position.is_none());
    }

    #[test]
    fn to_diag_entry_renders_level_and_position() {
        let err = RuntimeError::user_panic("boom".into(), "x.loft".into(), 3);
        let entry = err.to_diag_entry();
        assert_eq!(entry.level, Level::Error);
        assert_eq!(entry.file, "x.loft");
        assert_eq!(entry.line, 3);
        assert_eq!(entry.col, 1);
        assert!(entry.message.contains("boom"));
    }

    #[test]
    fn kinds_have_distinct_labels() {
        let kinds = [
            RuntimeErrorKind::DivideByZero,
            RuntimeErrorKind::IndexOutOfBounds { idx: 5, len: 3 },
            RuntimeErrorKind::NegativeIndex { idx: -1 },
            RuntimeErrorKind::NullDereference,
            RuntimeErrorKind::NarrowCastOverflow {
                value: 99_999,
                target: "i8",
            },
            RuntimeErrorKind::StackOverflow,
            RuntimeErrorKind::UserPanic {
                message: "u".into(),
            },
            RuntimeErrorKind::AssertionFailed {
                message: "a".into(),
            },
        ];
        let mut labels: Vec<&'static str> = kinds.iter().map(RuntimeErrorKind::label).collect();
        labels.sort_unstable();
        let n = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), n, "kind labels collided");
    }
}
