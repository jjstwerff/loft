// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I65 — Bytecode code generator

//! Bytecode generation stack — tracks the compile-time stack position.
//!
//! During codegen, [`Stack`] mirrors the runtime stack position so that
//! each operator knows where its operands will be.  `operator()` adjusts
//! the position by the operator's net stack effect.  [`Loop`] records the
//! stack depth at loop entry so `break`/`continue` can restore it.

use crate::data::{Context, Data};
use crate::data_store::ValueType;
use crate::ir_node::IrNode;
use crate::state::State;
use crate::variables;
use crate::variables::Function;

pub struct Loop {
    start: u32,
    stack: u16,
    breaks: Vec<u32>,
}

/// Stack information on variable positions and scopes to generate byte-code.
#[allow(dead_code)]
pub struct Stack<'a> {
    pub position: u16,
    pub data: &'a Data,
    pub function: Function,
    pub def_nr: u32,
    pub logging: bool,
    loops: Vec<Loop>,
    /// @PLN118 — vars that received an unconditional pre-build `OpFreeRef` on an
    /// owned reassignment (`generate_set`, the `owned_ref` path).  In a loop such a
    /// var is freed at the block-exit sweep AND re-freed by the next iteration's
    /// pre-build free; if the allocator reused the slot meanwhile, that re-free
    /// destroys a live value (the moros glb H4 UAF).  The block-exit `OpFreeRef`
    /// (`generate_call`) resets these — and only these — to the null sentinel, so
    /// the pre-build free no-ops.  Excludes retbuf/Database-reused locals (`off`),
    /// which never take a pre-build free and rely on keeping their DbRef.  Mirrors
    /// native's unconditional post-free reset (`generation/ops/ref_ops.rs`), scoped
    /// to the vars that actually need it.
    pub owned_reassigned: std::collections::HashSet<u16>,
}

impl<'a> Stack<'a> {
    pub fn new(function: Function, data: &'a Data, def_nr: u32, logging: bool) -> Stack<'a> {
        Stack {
            position: 0,
            data,
            def_nr,
            logging,
            loops: Vec::new(),
            function,
            owned_reassigned: std::collections::HashSet::new(),
        }
    }

    /// @PLAN53 cluster 2 / S4 — codegen mirror of `State::stack_step`: one
    /// eval-stack advance, always 8-rounded so the emitted `pos` operands
    /// match the aligned runtime `stack_pos` (compile-time and runtime move
    /// in lockstep, S1).
    #[inline]
    #[must_use]
    #[allow(clippy::unused_self)]
    pub fn step(&self, size: u16) -> u16 {
        variables::aligned_stack_step(u32::from(size)) as u16
    }

    /** Return the amount of space on stack is needed as calculated from code */
    pub fn size_code(&self, node: IrNode) -> u16 {
        match node.kind() {
            ValueType::Int | ValueType::Single => 4,
            ValueType::Long | ValueType::Float => 8,
            ValueType::Boolean | ValueType::Enum => 1,
            ValueType::Block => {
                let ops = node.as_block().operators();
                if ops.is_empty() {
                    0
                } else {
                    self.size_code(ops.get(ops.len() - 1))
                }
            }
            ValueType::Call => {
                variables::size(&self.data.def(node.call_to()).returned, &Context::Argument)
            }
            ValueType::If => self.size_code(node.if_then()),
            ValueType::Text => size_of::<&str>() as u16,
            ValueType::Var => variables::size(self.function.tp(node.var_nr()), &Context::Argument),
            ValueType::Insert => {
                let ops = node.insert_items();
                if ops.is_empty() {
                    0
                } else {
                    self.size_code(ops.get(ops.len() - 1))
                }
            }
            ValueType::Span => self.size_code(node.span_inner()),
            _ => 0,
        }
    }

    /// Compile-time distance from the current eval-stack top down to
    /// variable `v`'s frame slot — the `pos` operand emitted for every
    /// positional var op (`OpPutInt`, `OpVarRef`, `OpFreeText`, …).
    /// Both terms are `size()`-derived (see @PLAN53 cluster 2 / S1):
    /// `position` is advanced by `variables::size` as codegen tracks the
    /// eval stack, and `function.stack(v)` is the slot the allocator gave.
    ///
    /// # Panics
    /// When `v`'s slot exceeds the current eval position — which means
    /// either the variable was never assigned a slot (`stack_pos ==
    /// u16::MAX`, e.g. an allocator skipped it) or the eval stack sits
    /// below its frame slot.  Names the variable + both operands so the
    /// failure is self-explanatory instead of a bare
    /// "attempt to subtract with overflow".
    pub fn var_pos(&self, v: u16) -> u16 {
        let slot = self.function.stack(v);
        self.position.checked_sub(slot).unwrap_or_else(|| {
            panic!(
                "var_pos underflow in fn '{}': variable '{}' (v_nr={v}) slot={slot} \
                 exceeds eval position={} — variable has no assigned slot \
                 (stack_pos==u16::MAX, allocator skipped it?) or the eval stack \
                 is below its frame slot",
                self.data.def(self.def_nr).name,
                self.function.name(v),
                self.position,
            )
        })
    }

    pub fn operator(&mut self, d_nr: u32) {
        let d = self.data.def(d_nr);
        // @PLAN53 cluster 2 / S4: a mutable param / the return value each
        // occupies one stepped eval-stack slot (`put_stack`/`get_stack`
        // round to 8 in aligned mode), so sum the STEPPED sizes — Σ step,
        // never step(Σ) — to stay in lockstep with the per-value pushes.
        // `step` is identity when LOFT_ALIGN is off → V1 unchanged.
        let mut parameters = 0;
        for p in &d.attributes {
            if p.mutable {
                parameters += self.step(variables::size(&p.typedef, &Context::Argument));
            }
        }
        let ret = self.step(variables::size(&d.returned, &Context::Argument));
        assert!(
            self.position >= parameters,
            "Incorrect stack {} versus {parameters} in {} operator {d_nr}:{}",
            self.position,
            self.data.def(self.def_nr).name,
            self.data.def(d_nr).name
        );
        self.position -= parameters;
        self.position += ret;
    }

    pub fn add_op(&mut self, name: &str, state: &mut State) {
        let op_nr = self.data.def_nr(name);
        assert_ne!(op_nr, u32::MAX, "Unknown operator {name}");
        state.remember_stack(self.position);
        crate::state::emit_op(self.data.def(op_nr).op_code, state);
        self.operator(op_nr);
    }

    pub fn add_loop(&mut self, code_pos: u32) {
        self.loops.push(Loop {
            start: code_pos,
            stack: self.position,
            breaks: Vec::new(),
        });
    }

    pub fn end_loop(&mut self, state: &mut State) {
        let breaks = &self.loops.pop().unwrap().breaks;
        for b in breaks {
            state.code_put(*b, (i64::from(state.code_pos) - i64::from(*b) - 2) as i16);
        }
    }

    pub fn add_break(&mut self, code_pos: u32, loop_nr: u16) {
        let l = self.loops.len() - 1;
        self.loops[l - loop_nr as usize].breaks.push(code_pos);
    }

    pub fn get_loop(&self, loop_nr: u16) -> u32 {
        let l = self.loops.len() - 1;
        self.loops[l - loop_nr as usize].start
    }

    pub fn loop_position(&self, loop_nr: u16) -> u16 {
        let l = self.loops.len() - 1;
        self.loops[l - loop_nr as usize].stack
    }
}
