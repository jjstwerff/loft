// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Bytecode generation: lowers the `Value` IR tree into flat bytecode.
//!
//! Each function's IR (produced by the parser) is walked by
//! [`State::def_code`] which emits operator words into `State.bytecode`.
//! The emitted bytecode is a flat `Vec<u32>` indexed by code position;
//! each operator is one or more words (opcode + operands).
//!
//! Key helpers: `gen_set` (assignment), `gen_call` (function call),
//! `gen_format` (format strings), `gen_block` / `gen_if` / `gen_for`
//! (control flow).

use super::State;
use crate::data::{Block, Context, Data, I32, Type, Value};
use crate::stack::Stack;
#[cfg(debug_assertions)]
use crate::variables::Function;
use crate::variables::size;
use std::collections::HashSet;

/// Text-returning natives that accept a destination buffer instead of allocating one.
fn is_text_dest_native(name: &str) -> bool {
    matches!(
        name,
        "t_4text_replace" | "t_4text_to_lowercase" | "t_4text_to_uppercase"
    )
}

impl State {
    /**
    Define byte code for a function.
    # Panics
    when code cannot be output.
    */
    pub fn def_code(&mut self, def_nr: u32, data: &mut Data) {
        let logging = !data.def(def_nr).position.file.starts_with("default/");
        let console = false; //logging;
        let mut stack = Stack::new(data.def(def_nr).variables.clone(), data, def_nr, logging);
        if stack.data.def(def_nr).code == Value::Null {
            // B7 (2026-04-13): set up the same frame layout the regular
            // path uses, so `add_return` emits a correct OpReturn.  For
            // native fns declared as `pub fn name(...) -> T;` (no body),
            // the variables list is empty so `function.arguments()`
            // returns nothing — the regular path's `self.arguments` /
            // `stack.position` accumulator is never run, and we used to
            // emit `Return(ret=stale, discard=0)` with a leftover
            // `self.arguments` from the previous function.  This caused
            // native methods with struct-enum self (e.g.
            // `fn len(self: JsonValue) -> integer;`) to loop forever in
            // `Return(ret=12, value=4, discard=0)` because the runtime's
            // unwind landed past the saved return-PC slot.  Compute the
            // arg-frame size from the def's attributes (which exist for
            // native fns even when variables don't) and account for the
            // 4-byte return-PC slot.
            let mut args_size: u16 = 0;
            for attr in &stack.data.def(def_nr).attributes {
                args_size += size(&attr.typedef, &Context::Argument);
            }
            self.arguments = args_size;
            stack.position = args_size + 4; // args + return-address slot
            let start = self.code_pos;
            self.add_return(&mut stack, start);
            data.definitions[def_nr as usize].code_position = start;
            data.definitions[def_nr as usize].code_length = self.code_pos - start;
            return;
        }
        let is_empty_stub =
            matches!(&stack.data.def(def_nr).code, Value::Block(bl) if bl.operators.is_empty());
        // use arguments() instead of names map lookup — the names map may
        // redirect promoted text parameters to shadow locals.
        let args = stack.function.arguments();
        for v in &args {
            stack.function.set_stack_pos(*v, stack.position);
            stack.position += size(stack.function.tp(*v), &Context::Argument);
        }
        let start = self.code_pos;
        self.arguments = stack.position;
        stack.position += 4; // keep space for the code return address
        if is_empty_stub {
            self.add_return(&mut stack, start);
            data.definitions[def_nr as usize].code_position = start;
            data.definitions[def_nr as usize].code_length = self.code_pos - start;
            return;
        }
        if console {
            println!("{} ", stack.data.def(def_nr).header(stack.data, def_nr));
            stack.data.dump(def_nr);
        }
        let mut started = HashSet::new();
        for a in stack.data.def(def_nr).variables.arguments() {
            started.insert(a);
        }
        // Optional IR dump: set LOFT_IR=<name-filter> (or LOFT_IR=* for all user fns).
        if let Ok(filter) = std::env::var("LOFT_IR") {
            let fn_name = stack.data.def(def_nr).name.as_str();
            let want_all = filter.is_empty() || filter == "*";
            let matches = want_all || filter == fn_name || fn_name.contains(&*filter);
            if matches && logging {
                eprintln!("=== IR: {fn_name} ===");
                #[cfg(debug_assertions)]
                print_ir(&stack.data.def(def_nr).code, stack.data, &stack.function, 0);
                #[cfg(not(debug_assertions))]
                {
                    let mut w = Vec::new();
                    let mut vars = stack.function.clone();
                    let _ = stack.data.show_code(
                        &mut w,
                        &mut vars,
                        &stack.data.def(def_nr).code,
                        0,
                        true,
                    );
                    eprint!("{}", String::from_utf8_lossy(&w));
                }
                eprintln!();
                eprintln!("===");
            }
        }
        self.source = stack.data.def(def_nr).source;
        self.generate(&stack.data.def(def_nr).code, &mut stack, true);
        let mut stack_pos = Vec::new();
        let mut skip_free_vars = Vec::new();
        for v_nr in 0..stack.function.next_var() {
            stack_pos.push(stack.function.stack(v_nr));
            if stack.function.is_skip_free(v_nr) {
                skip_free_vars.push(v_nr);
            }
        }
        data.definitions[def_nr as usize].code_position = start;
        data.definitions[def_nr as usize].code_length = self.code_pos - start;
        if let Some(v) = self.calls.get(&def_nr) {
            let old = self.code_pos;
            for pos in v.clone() {
                // skip opcode(1) + d_nr(8) + args_size(2) to reach the i64 target
                self.code_pos = pos + 11;
                self.code_add(i64::from(start));
            }
            self.code_pos = old;
        }
        for (v_nr, pos) in stack_pos.into_iter().enumerate() {
            data.definitions[def_nr as usize]
                .variables
                .set_stack(v_nr as u16, pos);
        }
        // Propagate skip_free flags set during codegen (e.g. S34 Option A) so that
        // validate_slots can recognise intentional slot aliases and not report them
        // as conflicts.
        for v_nr in skip_free_vars {
            data.definitions[def_nr as usize]
                .variables
                .set_skip_free(v_nr);
        }
        #[cfg(any(debug_assertions, test))]
        crate::variables::validate_slots(
            &data.definitions[def_nr as usize].variables,
            data,
            def_nr,
        );
    }

    /**
    Generate the byte code equivalent of a function definition
    # Panics
    On not implemented Value constructions
    */
    pub(super) fn generate(&mut self, val: &Value, stack: &mut Stack, top: bool) -> Type {
        self.generate_depth += 1;
        assert!(
            self.generate_depth <= 1000,
            "expression nesting limit exceeded at depth {}",
            self.generate_depth
        );
        let result = self.generate_inner(val, stack, top);
        self.generate_depth -= 1;
        result
    }

    #[allow(clippy::too_many_lines)]
    fn generate_inner(&mut self, val: &Value, stack: &mut Stack, top: bool) -> Type {
        match val {
            Value::Int(value) => {
                stack.add_op("OpConstInt", self);
                self.code_add(i64::from(*value));
                I32.clone()
            }
            Value::Enum(value, tp) => {
                self.types.insert(self.code_pos, *tp);
                stack.add_op("OpConstEnum", self);
                self.code_add(*value);
                Type::Enum(0, false, Vec::new())
            }
            Value::Long(value) => {
                stack.add_op("OpConstInt", self);
                self.code_add(*value);
                crate::data::I64.clone()
            }
            Value::Single(value) => {
                stack.add_op("OpConstSingle", self);
                self.code_add(*value);
                Type::Single
            }
            Value::Float(value) => {
                stack.add_op("OpConstFloat", self);
                self.code_add(*value);
                Type::Float
            }
            Value::Keys(_) => {
                // Should be already part of the search request
                Type::Null
            }
            Value::Boolean(value) => {
                stack.add_op(
                    if *value {
                        "OpConstTrue"
                    } else {
                        "OpConstFalse"
                    },
                    self,
                );
                Type::Boolean
            }
            Value::Text(value) => self.gen_text(value, stack),
            Value::Var(v) => self.generate_var(stack, *v),
            Value::Set(v, value) => {
                self.generate_set(stack, *v, value);
                Type::Void
            }
            Value::Loop(lp) => self.gen_loop(lp, stack),
            Value::Insert(ops) => {
                for op in ops {
                    self.generate(op, stack, false);
                }
                Type::Void
            }
            Value::Break(loop_nr) => self.gen_break(*loop_nr, stack),
            Value::BreakWith(loop_nr, val) => {
                self.generate(val, stack, false);
                self.gen_break(*loop_nr, stack)
            }
            Value::Continue(loop_nr) => self.gen_continue(*loop_nr, stack),
            Value::If(test, t_val, f_val) => self.gen_if(test, t_val, f_val, stack),
            Value::Return(v) => self.gen_return(v, stack),
            Value::Block(bl) => self.generate_block(stack, bl, top),
            Value::Call(op, parameters) => self.generate_call(stack, *op, parameters),
            Value::CallRef(v_nr, args) => self.generate_call_ref(stack, *v_nr, args),
            Value::Null => {
                // Ignore, in use as the code on an else clause without code.
                Type::Void
            }
            Value::Drop(val) => self.gen_drop(val, stack),
            Value::Iter(_, _, _, _) => {
                panic!("Should have rewritten {val:?}");
            }
            Value::Line(line) => {
                self.line_numbers.insert(self.code_pos, *line);
                if let Some(&lib_nr) = self.library_names.get("OpClearScratch") {
                    stack.add_op("OpStaticCall", self);
                    self.code_add(lib_nr);
                }
                Type::Void
            }
            Value::Tuple(elems) => {
                // T1.4: generate each element onto contiguous stack slots.
                let mut types = Vec::new();
                for e in elems {
                    let t = self.generate(e, stack, false);
                    types.push(t);
                }
                Type::Tuple(types)
            }
            Value::Parallel(arms) => {
                self.gen_parallel(arms, stack);
                Type::Void
            }
            Value::TupleGet(var_nr, elem_idx) => {
                let tuple_tp = stack.function.tp(*var_nr).clone();
                // T1.5: RefVar(Tuple) — read element through the DbRef using OpGetInt/etc.
                if let Type::RefVar(ref inner) = tuple_tp
                    && let Type::Tuple(ref elems) = **inner
                {
                    let idx = *elem_idx as usize;
                    let elem_tp = elems[idx].clone();
                    let offsets = crate::data::element_offsets(elems);
                    let elem_offset = offsets[idx] as u16;
                    let var_pos = stack.position - stack.function.stack(*var_nr);
                    let code_pos = self.code_pos;
                    stack.add_op("OpVarRef", self);
                    self.code_add(var_pos);
                    match &elem_tp {
                        Type::Integer(_, _, _) | Type::Function(_, _, _) => {
                            stack.add_op("OpGetInt", self);
                        }
                        Type::Float => stack.add_op("OpGetFloat", self),
                        Type::Single => stack.add_op("OpGetSingle", self),
                        Type::Character => stack.add_op("OpGetCharacter", self),
                        _ => panic!("RefTupleGet: unsupported element type {elem_tp:?}"),
                    }
                    self.code_add(elem_offset);
                    return self.insert_types(elem_tp, code_pos, stack);
                }
                // T1.4: read element elem_idx from tuple variable var_nr.
                let Type::Tuple(ref elems) = tuple_tp else {
                    panic!("TupleGet on non-tuple variable");
                };
                let idx = *elem_idx as usize;
                let elem_tp = elems[idx].clone();
                let offsets = crate::data::element_offsets(elems);
                let elem_offset = offsets[idx] as u16;
                // The element is at tuple_var_stack_pos + elem_offset.
                // Compute distance from current stack top to that position.
                let tuple_var_pos = stack.function.stack(*var_nr);
                let elem_abs_pos = tuple_var_pos + elem_offset;
                let var_pos = stack.position - elem_abs_pos;
                let code_pos = self.code_pos;
                match &elem_tp {
                    Type::Integer(_, _, _) | Type::Function(_, _, _) => {
                        stack.add_op("OpVarInt", self);
                    }
                    Type::Boolean => stack.add_op("OpVarBool", self),
                    Type::Float => stack.add_op("OpVarFloat", self),
                    Type::Single => stack.add_op("OpVarSingle", self),
                    Type::Character => stack.add_op("OpVarCharacter", self),
                    Type::Enum(_, false, _) => stack.add_op("OpVarEnum", self),
                    Type::Text(_) => stack.add_op("OpArgText", self),
                    Type::Reference(c, _) | Type::Enum(c, true, _) => {
                        self.types
                            .insert(self.code_pos, stack.data.def(*c).known_type);
                        stack.add_op("OpVarRef", self);
                    }
                    _ => panic!("TupleGet: unsupported element type {elem_tp:?}"),
                }
                self.code_add(var_pos);
                self.insert_types(elem_tp.clone(), code_pos, stack)
            }
            Value::Yield(inner) => {
                // CO1.3c: emit the yielded expression, then OpCoroutineYield.
                let t = self.generate(inner, stack, false);
                let value_size = crate::variables::size(&t, &crate::data::Context::Argument);
                stack.add_op("OpCoroutineYield", self);
                self.code_add(value_size);
                // CO1.3d: OpCoroutineYield suspends and transfers the value to the caller.
                // The evaluation stack is empty again on resume, so undo the push.
                stack.position -= value_size;
                Type::Void
            }
            Value::TuplePut(var_nr, elem_idx, value) => {
                let tuple_tp = stack.function.tp(*var_nr).clone();
                // T1.5: RefVar(Tuple) — write element through the DbRef using OpSetInt/etc.
                if let Type::RefVar(ref inner) = tuple_tp
                    && let Type::Tuple(ref elems) = **inner
                {
                    let idx = *elem_idx as usize;
                    let elem_tp = elems[idx].clone();
                    let offsets = crate::data::element_offsets(elems);
                    let elem_offset = offsets[idx] as u16;
                    let var_pos = stack.position - stack.function.stack(*var_nr);
                    stack.add_op("OpVarRef", self);
                    self.code_add(var_pos);
                    self.generate(value, stack, false);
                    match &elem_tp {
                        Type::Integer(_, _, _) | Type::Function(_, _, _) => {
                            stack.add_op("OpSetInt", self);
                        }
                        Type::Float => stack.add_op("OpSetFloat", self),
                        Type::Character => stack.add_op("OpSetCharacter", self),
                        _ => panic!("RefTuplePut: unsupported element type {elem_tp:?}"),
                    }
                    self.code_add(elem_offset);
                    return Type::Void;
                }
                // T1.4: write to element elem_idx of tuple variable var_nr.
                let Type::Tuple(ref elems) = tuple_tp else {
                    panic!("TuplePut on non-tuple variable");
                };
                let idx = *elem_idx as usize;
                let elem_tp = elems[idx].clone();
                let offsets = crate::data::element_offsets(elems);
                let elem_offset = offsets[idx] as u16;
                // Generate the value to write.
                self.generate(value, stack, false);
                // Compute distance from stack top to the element's position.
                let tuple_var_base = stack.function.stack(*var_nr);
                let elem_abs_pos = tuple_var_base + elem_offset;
                let var_pos = stack.position - elem_abs_pos;
                match &elem_tp {
                    Type::Integer(_, _, _) | Type::Function(_, _, _) => {
                        stack.add_op("OpPutInt", self);
                    }
                    Type::Boolean => stack.add_op("OpPutBool", self),
                    Type::Float => stack.add_op("OpPutFloat", self),
                    Type::Single => stack.add_op("OpPutSingle", self),
                    Type::Character => stack.add_op("OpPutCharacter", self),
                    Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
                    Type::Text(_) => stack.add_op("OpAppendText", self),
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => {
                        stack.add_op("OpPutRef", self);
                    }
                    _ => panic!("TuplePut: unsupported element type {elem_tp:?}"),
                }
                self.code_add(var_pos);
                Type::Void
            }
            Value::FnRef(d_nr, clos_var, fn_type) => {
                // Construct 16-byte fn-ref on stack: push d_nr (4B) then closure DbRef (12B).
                // Uses existing OpConstInt + OpVarRef — no new opcode needed.
                // add_op → operator() already advances stack.position; no manual +4/+12 needed.
                stack.add_op("OpConstInt", self);
                self.code_add(i64::from(*d_nr));
                // clos_pos computed after ConstInt advanced stack.position by 8 (post-2c).
                let clos_pos = stack.position - stack.function.stack(*clos_var);
                stack.add_op("OpVarRef", self);
                self.code_add(clos_pos);
                *fn_type.clone()
            }
        }
    }

    pub(super) fn gen_text(&mut self, value: &str, stack: &mut Stack) -> Type {
        if value.len() < 256 {
            stack.add_op("OpConstText", self);
            self.code_add_str(value);
        } else {
            // Store long strings in CONST_STORE via Store::set_str().
            let const_store = &mut self.database.allocations[crate::database::CONST_STORE as usize];
            let rec = const_store.set_str(value);
            stack.add_op("OpConstStoreText", self);
            self.code_add(i64::from(rec));
            // set_str stores length at (rec, 4); text bytes start at (rec, 8).
            // We encode the record position; the opcode reads length from the store.
            self.code_add(0i64); // pos offset within the record (length is at rec+4)
        }
        Type::Text(Vec::new())
    }

    pub(super) fn gen_loop(&mut self, lp: &crate::data::Block, stack: &mut Stack) -> Type {
        stack.add_loop(self.code_pos);
        let pos = self.code_pos;
        for v in &lp.operators {
            self.generate(v, stack, false);
        }
        self.clear_stack(stack, 0);
        stack.add_op("OpGotoWord", self);
        self.code_add((i64::from(pos) - i64::from(self.code_pos) - 2) as i16);
        stack.end_loop(self);
        Type::Void
    }

    pub(super) fn gen_break(&mut self, loop_nr: u16, stack: &mut Stack) -> Type {
        let old_pos = stack.position;
        self.clear_stack(stack, loop_nr);
        stack.add_op("OpGotoWord", self);
        stack.add_break(self.code_pos, loop_nr);
        self.code_add(0i16); // temporary value to the end of the loop
        stack.position = old_pos;
        Type::Void
    }

    pub(super) fn gen_continue(&mut self, loop_nr: u16, stack: &mut Stack) -> Type {
        let old_pos = stack.position;
        self.clear_stack(stack, loop_nr);
        stack.add_op("OpGotoWord", self);
        self.code_add((i64::from(stack.get_loop(loop_nr)) - i64::from(self.code_pos) - 2) as i16);
        stack.position = old_pos;
        Type::Void
    }

    pub(super) fn gen_if(
        &mut self,
        test: &Value,
        t_val: &Value,
        f_val: &Value,
        stack: &mut Stack,
    ) -> Type {
        self.generate(test, stack, false);
        stack.add_op("OpGotoFalseWord", self);
        let code_step = self.code_pos;
        self.code_add(0i16); // temp step
        let true_pos = self.code_pos;
        let stack_pos = stack.position;
        let tp = self.generate(t_val, stack, false);
        let true_stack = stack.position;
        if *f_val == Value::Null {
            self.code_put(code_step, (self.code_pos - true_pos) as i16); // actual step
            // P136: when the true branch diverges (return/break/continue, possibly
            // wrapped by scopes.rs in Insert/Block), execution only reaches the
            // join point via the goto-false path — where runtime stack_pos equals
            // the pre-if `stack_pos`, not `true_stack`. Without this reset, every
            // subsequent Var/Put encodes a wrong offset and writes corrupt the
            // return-address slot, eventually overflowing the stack store.
            if is_divergent(t_val) {
                stack.position = stack_pos;
            }
        } else {
            stack.add_op("OpGotoWord", self);
            let end = self.code_pos;
            self.code_add(0i16); // temp end
            let false_pos = self.code_pos;
            self.code_put(code_step, (self.code_pos - true_pos) as i16); // actual step
            stack.position = stack_pos;
            let fp = self.generate(f_val, stack, false);
            let false_stack = stack.position;
            // B5: when both arms are non-divergent but exit at different stack
            // levels (e.g. match arms with different local allocations), the
            // shorter arm's result value sits at a lower stack position than
            // the longer arm's.  Rather than padding (which would bury the
            // result under garbage), emit the join point at the SHORTER arm's
            // level and make the longer arm discard its extra bytes before
            // reaching the join point (its result is already on top of stack).
            if !is_divergent(t_val) && !is_divergent(f_val) && true_stack != false_stack {
                let target = true_stack.min(false_stack);
                if false_stack > target {
                    // Shrink the false arm's stack to match the true arm.
                    let excess = false_stack - target;
                    stack.add_op("OpFreeStack", self);
                    let ret_size = size(&stack.data.def(stack.def_nr).returned, &Context::Argument);
                    self.code_add(ret_size as u8);
                    self.code_add(excess + ret_size);
                    stack.position = target;
                }
                self.code_put(end, (self.code_pos - false_pos) as i16);
                stack.position = target;
            } else {
                self.code_put(end, (self.code_pos - false_pos) as i16);
                // when one branch diverges (return/break/continue), use the
                // other branch's stack position. The divergent branch exits the
                // scope so its stack delta is irrelevant at the join point.
                if is_divergent(t_val) {
                    stack.position = false_stack;
                } else if is_divergent(f_val) {
                    stack.position = true_stack;
                }
            }
            if matches!(tp, Type::Never) {
                return fp;
            }
        }
        tp
    }

    /// Generate a fn-ref assignment value, ensuring every branch in an if-else
    /// expression produces a full 16-byte fn-ref slot ([d_nr 4B][closure DbRef 12B]).
    /// A plain branch only pushes 4 bytes (d_nr via OpConstInt); OpNullRefSentinel pads
    /// to 16 bytes.  For if-else, the sentinel must be emitted *inside* each branch so
    /// both paths reach the join point with the same stack delta.
    fn gen_fn_ref_value(&mut self, value: &Value, stack: &mut Stack) {
        if let Value::If(test, t_val, f_val) = value {
            self.generate(test, stack, false);
            stack.add_op("OpGotoFalseWord", self);
            let code_step = self.code_pos;
            self.code_add(0i16);
            let true_pos = self.code_pos;
            let stack_pos = stack.position;
            self.gen_fn_ref_value(t_val, stack);
            stack.add_op("OpGotoWord", self);
            let end = self.code_pos;
            self.code_add(0i16);
            let false_pos = self.code_pos;
            self.code_put(code_step, (self.code_pos - true_pos) as i16);
            stack.position = stack_pos;
            self.gen_fn_ref_value(f_val, stack);
            self.code_put(end, (self.code_pos - false_pos) as i16);
        } else {
            let before = stack.position;
            self.generate(value, stack, false);
            if stack.position - before < 16 {
                stack.add_op("OpNullRefSentinel", self);
            }
        }
    }

    pub(super) fn gen_return(&mut self, v: &Value, stack: &mut Stack) -> Type {
        self.generate(v, stack, false);
        let return_type = &stack.data.def(stack.def_nr).returned;
        // CO1.3c: generator functions use OpCoroutineReturn instead of OpReturn.
        if matches!(return_type, Type::Iterator(_, _)) {
            // For generators, `return` means exhaust — push null of the yield type.
            let yield_size = if let Type::Iterator(inner, _) = return_type {
                size(inner, &Context::Argument)
            } else {
                0
            };
            stack.add_op("OpCoroutineReturn", self);
            self.code_add(yield_size);
        } else {
            if return_type != &Type::Void {
                let ret_nr = stack.data.type_def_nr(return_type);
                let known = stack.data.def(ret_nr).known_type;
                self.types.insert(self.code_pos, known);
            }
            stack.add_op("OpReturn", self);
            self.code_add(self.arguments);
            self.code_add(size(return_type, &Context::Argument) as u8);
            self.code_add(stack.position);
        }
        Type::Void
    }

    pub(super) fn gen_drop(&mut self, val: &Value, stack: &mut Stack) -> Type {
        self.generate(val, stack, false);
        // get all variables of the current scope.
        let size = stack.size_code(val);
        if size > 0 {
            stack.add_op("OpFreeStack", self);
            self.code_add(0u8);
            self.code_add(size);
        }
        stack.position -= size;
        Type::Void
    }

    /// Generate bytecode for `parallel { arm1; arm2; ... }`.
    ///
    /// Current implementation: sequential execution inline.
    /// The opcodes are emitted for future threading support but act as noops.
    /// Arms run inline in sequence, each dropping its result value.
    /// Generate bytecode for `parallel { arm1; arm2; ... }`.
    ///
    /// Layout:
    ///   `OpParallelBegin(n)`
    ///   `OpParallelArm(off0)` `OpParallelArm(off1)` ...
    ///   `OpParallelJoin` — main thread blocks here
    ///   `OpGotoWord(skip_arms)` — main thread skips arm code
    ///   [arm0 code] `OpReturn`
    ///   [arm1 code] `OpReturn`
    ///   [continue]
    pub(super) fn gen_parallel(&mut self, arms: &[Value], stack: &mut Stack) {
        let n = arms.len();
        stack.add_op("OpParallelBegin", self);
        self.code_add(n as u8);
        // OpParallelArm(offset) placeholders
        let mut arm_offset_positions = Vec::with_capacity(n);
        for _ in 0..n {
            stack.add_op("OpParallelArm", self);
            arm_offset_positions.push(self.code_pos);
            self.code_add(0u16);
        }
        // OpParallelJoin — join_pos is code_pos after this opcode
        stack.add_op("OpParallelJoin", self);
        let join_pos = self.code_pos;
        // OpGotoWord past all arm code — main thread skips
        stack.add_op("OpGotoWord", self);
        let goto_skip_pos = self.code_pos;
        self.code_add(0i16);
        let goto_skip_base = self.code_pos;
        // Emit each arm as a separate region ending with OpReturn
        for (i, arm) in arms.iter().enumerate() {
            let arm_start = self.code_pos;
            let offset = (arm_start - join_pos) as u16;
            self.code_put(arm_offset_positions[i], offset);
            self.gen_drop(arm, stack);
            // OpReturn — arm is a void "function"
            stack.add_op("OpReturn", self);
            self.code_add(0u16); // arguments = 0
            self.code_add(0u8); // return_size = 0
            self.code_add(stack.position);
        }
        // Patch skip goto
        let end_pos = self.code_pos;
        self.code_put(goto_skip_pos, (end_pos - goto_skip_base) as i16);
    }

    pub(super) fn gen_set_first_text(&mut self, stack: &mut Stack, v: u16, value: &Value) {
        stack.add_op("OpText", self);
        stack.position += super::size_str() as u16;
        if let Value::Text(s) = value {
            if !s.is_empty() {
                self.set_var(stack, v, value);
            }
        } else {
            self.set_var(stack, v, value);
        }
    }

    pub(super) fn gen_set_first_ref_null(&mut self, stack: &mut Stack, v: u16) {
        let dep = match stack.function.tp(v).clone() {
            Type::Reference(_, d) | Type::Enum(_, _, d) => d,
            _ => Vec::new(),
        };
        if dep.is_empty() {
            if stack.function.is_inline_ref(v) {
                // Inline-ref temporaries must not allocate a database store at null-init
                // time.  A real store is assigned later via OpPutRef when the method
                // returns.  OpNullRefSentinel places DbRef{store_nr:u16::MAX} in the
                // slot; Stores::free treats it as a no-op if the var is never assigned.
                stack.add_op("OpNullRefSentinel", self);
            } else {
                stack.add_op("OpConvRefFromNull", self);
            }
        } else {
            // Pre-init a borrowed Reference with a null-state DbRef pointing into dep's slot.
            // The DbRef uses stack_cur.store_nr (the stack-frame store) — it is NOT a valid
            // data-store pointer and must be overwritten by OpPutRef before any field access.
            // See State::create_stack() and ASSIGNMENT.md §"Option A sub-option 3" for details.
            //
            // Argument: pos = (stack.position before this op) - dep[0].stack_pos
            //   → result.pos = stack_cur.pos + dep[0].stack_pos (points into dep's slot)
            let dep_pos = stack.function.stack(dep[0]);
            let before_stack = stack.position;
            if dep_pos > before_stack {
                // Dependency not yet on the stack — use a null sentinel instead of
                // CreateStack.  The variable will be overwritten by OpPutRef before
                // any field access (e.g. loop variable assigned from iterator next).
                stack.add_op("OpNullRefSentinel", self);
            } else {
                stack.add_op("OpCreateStack", self);
                let after_stack = stack.position - size_of::<crate::keys::DbRef>() as u16;
                self.code_add(after_stack - dep_pos);
            }
        }
    }

    pub(super) fn gen_set_first_tuple_null(&mut self, stack: &mut Stack, v: u16) {
        let Type::Tuple(elems) = stack.function.tp(v).clone() else {
            return;
        };
        for elem in &elems {
            match elem {
                Type::Integer(_, _, _) | Type::Function(_, _, _) => {
                    stack.add_op("OpConstInt", self);
                    self.code_add(0i64);
                }
                Type::Boolean => {
                    stack.add_op("OpConstFalse", self);
                }
                Type::Single => {
                    stack.add_op("OpConstSingle", self);
                    self.code_add(0.0f32);
                }
                Type::Float => {
                    stack.add_op("OpConstFloat", self);
                    self.code_add(0.0f64);
                }
                Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _) => {
                    // T1.8c: use NullRefSentinel (no store allocation) for tuple
                    // reference elements.  The element will be overwritten by PutRef
                    // or CopyRecord during destructuring; a real store is not needed
                    // at null-init time.  Using OpConvRefFromNull here would leak a
                    // store because the tuple scope-exit skip (scopes.rs:587) never
                    // frees tuple elements.
                    stack.add_op("OpNullRefSentinel", self);
                }
                Type::Text(_) => {
                    stack.add_op("OpConvTextFromNull", self);
                }
                other => panic!("gen_set_first_tuple_null: unsupported element type {other:?}"),
            }
        }
    }

    pub(super) fn gen_set_first_vector_null(&mut self, stack: &mut Stack, v: u16) {
        if let Type::Vector(elm_tp, dep) = stack.function.tp(v).clone() {
            // skip_free variables are match-arm bindings that borrow from the
            // subject — don't allocate a store, just push a null sentinel.
            if stack.function.is_skip_free(v) {
                stack.add_op("OpNullRefSentinel", self);
            } else if stack.function.is_inline_ref(v) {
                // Lift temporaries (scopes.rs scan_args) are always assigned
                // from a call result before first read; the null-init only
                // needs to reserve the slot and leave a null DbRef so an
                // OpFreeRef along an un-taken path is a no-op.  Skipping the
                // full pre-alloc avoids a dangling vector store (P54-B/Q2
                // chain-leak fix).
                stack.add_op("OpNullRefSentinel", self);
            } else if dep.is_empty() {
                // TODO move this convoluted implementation to a new operator.
                stack.add_op("OpConvRefFromNull", self);
                stack.add_op("OpDatabase", self);
                self.code_add(size_of::<crate::keys::DbRef>() as u16);
                let name = format!("main_vector<{}>", elm_tp.name(stack.data));
                let known = stack.data.name_type(&name, self.source);
                debug_assert_ne!(
                    known,
                    u16::MAX,
                    "Incomplete type {name} in {}",
                    stack.function.name
                );
                self.code_add(known);
                stack.add_op("OpVarRef", self);
                self.code_add(size_of::<crate::keys::DbRef>() as u16);
                stack.add_op("OpConstInt", self);
                self.code_add(0i64);
                // Vector header length field is 4 bytes (u32).  Post-2c
                // `OpSetInt` writes 8 bytes and overflows into adjacent
                // storage.  Use `OpSetInt4` to write only 4 bytes.
                stack.add_op("OpSetInt4", self);
                self.code_add(4u16);
                stack.add_op("OpCreateStack", self);
                self.code_add(size_of::<crate::keys::DbRef>() as u16);
                stack.add_op("OpConstInt", self);
                self.code_add(12i64);
                stack.add_op("OpSetByte", self);
                self.code_add(4u16);
                self.code_add(0u16);
            } else {
                // Same pre-init logic as for borrowed Reference types above:
                // OpCreateStack produces a stack-frame DbRef pointing into dep's slot.
                // Must be overwritten by OpPutRef before any field access.
                stack.add_op("OpCreateStack", self);
                let dep_pos = stack.function.stack(dep[0]);
                let before_stack = stack.position - size_of::<crate::keys::DbRef>() as u16;
                self.code_add(before_stack - dep_pos);
            }
        }
    }

    /// Emit a null sentinel for the given type onto the stack.
    /// Used when Value::Null appears in a typed context (e.g. function argument).
    fn emit_typed_null(&mut self, stack: &mut Stack, tp: &Type) {
        match tp {
            Type::Text(_) => {
                stack.add_op("OpConvTextFromNull", self);
            }
            Type::Reference(_, _) | Type::Enum(_, true, _) => {
                stack.add_op("OpConvRefFromNull", self);
            }
            Type::Integer(_, _, _) | Type::Character => {
                stack.add_op("OpConstInt", self);
                self.code_add(i64::MIN);
            }
            Type::Float => {
                stack.add_op("OpConstFloat", self);
                self.code_add(f64::NAN);
            }
            Type::Single => {
                stack.add_op("OpConstSingle", self);
                self.code_add(f32::NAN.to_bits());
            }
            Type::Boolean => {
                stack.add_op("OpConstInt", self);
                self.code_add(i64::MIN);
            }
            _ => {
                // For other types, push a zero-filled DbRef as a generic null.
                stack.add_op("OpNullRefSentinel", self);
            }
        }
    }

    /// Adjust the slot position for a first-assignment variable.
    /// Case 1: pre-assigned above TOS → move down. Case 2: large type below TOS →
    /// override only if no child-scope overlap (A13 guard).
    #[allow(clippy::too_many_lines)]
    pub(super) fn generate_set(&mut self, stack: &mut Stack, v: u16, value: &Value) {
        self.vars.insert(self.code_pos, v);
        // Zero-sized variables (null-typed) have no stack storage.
        if size(stack.function.tp(v), &Context::Variable) == 0 {
            stack.function.set_stack_allocated(v);
            return;
        }
        let pos = stack.function.stack(v);
        assert!(
            pos != u16::MAX,
            "variable '{}' never assigned a slot",
            stack.function.name(v)
        );
        if stack.function.is_stack_allocated(v) {
            // Reassignment — variable already on the stack.
            if matches!(stack.function.tp(v), Type::Text(_)) {
                let var_pos = stack.position - pos;
                stack.add_op("OpClearText", self);
                self.code_add(var_pos);
            }
            // Free the old store before reassigning an owned Reference.
            // Only safe when the variable truly owns its store (dep empty).
            // The dep was cleared on first assignment only when codegen will
            // deep-copy (gen_set_first_ref_call_copy). O-B2 adopted stores
            // keep their dep, so this won't fire for them.
            if matches!(
                stack.function.tp(v),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            ) && stack.function.tp(v).depend().is_empty()
            {
                let free_pos = stack.position - stack.function.stack(v);
                stack.add_op("OpVarRef", self);
                self.code_add(free_pos);
                stack.add_op("OpFreeRef", self);

                // when the value is a call with visible Ref
                // params, the callee returns via a hidden __ref_N that is
                // reused across calls.  OpPutRef would alias v with __ref_N;
                // the next FreeRef would free __ref_N's store → use-after-free.
                // Deep-copy into a fresh store instead; do NOT free the source
                // so __ref_N stays valid for the next call.
                if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
                    && !stack.function.is_argument(v)
                    && let Value::Call(fn_nr, _) = value
                    && stack.data.def(*fn_nr).name.starts_with("n_")
                    && stack.data.def(*fn_nr).code != Value::Null
                    && stack.data.def(*fn_nr).attributes.iter().any(|a| {
                        !a.hidden
                            && matches!(a.typedef, Type::Reference(_, _) | Type::Enum(_, true, _))
                    })
                {
                    let tp_nr = stack.data.def(d_nr).known_type;
                    // Allocate fresh store, put it in v's slot.
                    stack.add_op("OpConvRefFromNull", self);
                    stack.add_op("OpDatabase", self);
                    self.code_add(size_of::<crate::keys::DbRef>() as u16);
                    self.code_add(tp_nr);
                    let var_pos = stack.position - stack.function.stack(v);
                    stack.add_op("OpPutRef", self);
                    self.code_add(var_pos);
                    // Call, deep-copy into v.  Free the source only if the
                    // callee has no hidden Ref params (the source is a fresh
                    // store from O-B2 adoption).  When hidden Ref params exist,
                    // the source IS __ref_N which is reused across calls.
                    let has_hidden_ref = stack
                        .data
                        .def(*fn_nr)
                        .attributes
                        .iter()
                        .any(|a| a.hidden && a.typedef.heap_dep().is_some());
                    let copy_nr = stack.data.def_nr("OpCopyRecord");
                    // P181: same gate as `gen_set_first_ref_call_copy`.
                    // A borrowed-view return (non-empty dep chain) must
                    // NOT have its source freed — the "source" is a slice
                    // of the caller-owned arg's store.
                    let is_borrowed_view = !stack.data.def(*fn_nr).returned.depend().is_empty();
                    let tp_val = if has_hidden_ref || is_borrowed_view {
                        i32::from(tp_nr)
                    } else {
                        i32::from(tp_nr) | 0x8000
                    };
                    let copy_val = Value::Call(
                        copy_nr,
                        vec![value.clone(), Value::Var(v), Value::Int(tp_val)],
                    );
                    // P155: bracket the call + deep-copy with n_set_store_lock
                    // on every Ref/Vector/Enum arg (like gen_set_first_ref_call_copy
                    // already does for first-assignments).  Without this, when the
                    // callee returns a DbRef aliased with a caller arg, OpCopyRecord's
                    // 0x8000 free-source flag frees the caller's arg store — later
                    // uses of that arg SIGSEGV in OpGetVector.  The first-assignment
                    // path hit this bug too until the P143 fix added the bracket;
                    // the reassignment path needed the same treatment.
                    let ref_args: Vec<u16> = if let Value::Call(fn_nr, args) = value {
                        let attrs = stack.data.def(*fn_nr).attributes.clone();
                        args.iter()
                            .enumerate()
                            .filter_map(|(i, arg)| {
                                let tp = attrs.get(i).map(|a| &a.typedef)?;
                                if !matches!(
                                    tp,
                                    Type::Reference(_, _)
                                        | Type::Vector(_, _)
                                        | Type::Enum(_, true, _)
                                ) {
                                    return None;
                                }
                                if let Value::Var(av) = arg {
                                    Some(*av)
                                } else {
                                    None
                                }
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let lock_fn = stack.data.def_nr("n_set_store_lock");
                    for av in &ref_args {
                        let lock =
                            Value::Call(lock_fn, vec![Value::Var(*av), Value::Boolean(true)]);
                        self.generate(&lock, stack, false);
                    }
                    self.generate(&copy_val, stack, false);
                    for av in &ref_args {
                        let unlock =
                            Value::Call(lock_fn, vec![Value::Var(*av), Value::Boolean(false)]);
                        self.generate(&unlock, stack, false);
                    }
                    return;
                }
            }
            self.set_var(stack, v, value);
        } else {
            // First allocation — slot pre-assigned by assign_slots.
            #[cfg(debug_assertions)]
            assert!(
                !ir_contains_var(value, v),
                "[generate_set] first-assignment of '{}' (var_nr={v}) in '{}' contains \
                 a Var({v}) self-reference — storage not yet allocated, will produce a \
                 garbage DbRef at runtime. This is a parser bug. value={value:?}",
                stack.function.name(v),
                stack.data.def(stack.def_nr).name,
            );
            stack.function.set_stack_allocated(v);
            if pos >= stack.position {
                self.gen_set_first_at_tos(stack, v, value);
            } else {
                // Slot is below TOS: zone1 variable reusing a dead slot.
                if matches!(stack.function.tp(v), Type::Tuple(_)) && *value == Value::Null {
                    return;
                }
                // Large types below TOS need initialization at TOS first,
                // then OpPut to store at the slot. set_var handles this for
                // reassignment but not for first assignment — use
                // gen_set_first_at_tos which handles init properly.
                // It will assert pos == TOS, so we must accept that for
                // large types below TOS this assertion may fire.
                // The old adjust_first_assignment_slot moved these to TOS;
                // we now route them through the same path.
                if matches!(
                    stack.function.tp(v),
                    Type::Text(_)
                        | Type::Reference(_, _)
                        | Type::Vector(_, _)
                        | Type::Enum(_, true, _)
                ) {
                    self.gen_set_first_at_tos(stack, v, value);
                } else {
                    self.set_var(stack, v, value);
                }
            }
        }
    }

    /// First assignment at current TOS — dispatch by variable type.
    fn gen_set_first_at_tos(&mut self, stack: &mut Stack, v: u16, value: &Value) {
        let pos = stack.function.stack(v);
        // When pos < TOS (large type reusing dead slot below TOS), move
        // the variable's slot to TOS so the init opcode writes correctly.
        if pos < stack.position {
            stack.function.set_stack_pos(v, stack.position);
        }
        // PROBLEMS.md #139: when the slot allocator reserved a slot above
        // TOS (because a zone-1 byte-sized variable — plain enum or
        // boolean — was placed at a fixed slot inside the zone-2 frontier
        // without advancing codegen's TOS through it), bump the runtime
        // stack pointer with an OpReserveFrame so slot == TOS for the
        // init opcode below.  The reserved bytes cover the zone-1 var's
        // slot; the zone-1 var was already written via OpPutEnum/etc. so
        // the bytes contain live data, not garbage.
        else if pos > stack.position {
            let gap = pos - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(gap);
            stack.position += gap;
        }
        let pos = stack.function.stack(v);
        assert!(
            pos == stack.position,
            "[gen_set_first_at_tos] '{}' in '{}': slot={pos} but TOS={} — \
             caller must ensure TOS matches the variable's slot before calling",
            stack.function.name(v),
            stack.data.def(stack.def_nr).name,
            stack.position,
        );
        // Slot is at current TOS — use direct placement (same as old claim() path).
        // Large types (text, refs, vectors) always land here; non-reusing primitives too.
        if matches!(*stack.function.tp(v), Type::Text(_)) {
            self.gen_set_first_text(stack, v, value);
        } else if matches!(
            stack.function.tp(v),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && *value == Value::Null
        {
            self.gen_set_first_ref_null(stack, v);
        } else if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
            && let Value::Call(op_nr, _) = value
            && stack.data.def(*op_nr).name == "OpCopyRecord"
        {
            // The first assignment of a Reference variable being copied from another:
            // allocate a fresh store, initialize the struct record, then copy the data.
            self.gen_set_first_ref_copy(stack, d_nr, value);
        } else if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
            && let Value::Var(src) = value
            && let Type::Reference(src_d_nr, _) = stack.function.tp(*src)
            && d_nr == *src_d_nr
        {
            // First assignment `d = c` where both are owned References to the same struct:
            // give d its own independent record by allocating storage and copying c's data.
            self.gen_set_first_ref_var_copy(stack, v, *src, d_nr);
        } else if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
            && let Value::TupleGet(_, _) = value
        {
            // T1.8c: tuple destructuring `(q1, q2) = expr` — when an element
            // is Type::Reference, deep-copy the record to avoid aliasing.
            self.gen_set_first_ref_tuple_copy(stack, v, value, d_nr);
        } else if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
            && let Value::Call(fn_nr, _) = value
            && stack.data.def(*fn_nr).name.starts_with("n_")
            && stack.data.def(*fn_nr).code != Value::Null
        {
            // when the callee has no visible Reference params, the
            // caller and callee share __ref_N's store. No deep copy needed
            // since OpAppendVector in handle_field already deep-copies vector
            // field data into the struct's store during construction.
            let has_ref_params = stack.data.def(*fn_nr).attributes.iter().any(|a| {
                !a.hidden && matches!(a.typedef, Type::Reference(_, _) | Type::Enum(_, true, _))
            });
            if has_ref_params {
                self.gen_set_first_ref_call_copy(stack, v, value, d_nr);
            } else {
                // P150: previously this branch marked every __ref_* arg
                // as skip_free under the assumption that the callee always
                // wrote into the placeholder (so v and __ref_N shared a
                // store, freeing both = double-free).  But when the callee
                // returns a fresh store (early-return through a constructor
                // call, or T.parse(text) that allocates fresh — both
                // shapes used by `lib/moros_map/src/moros_map.loft`'s
                // `map_from_json`), the placeholder ConvRefFromNull alloc
                // is orphaned: __ref_N's slot still points to the
                // placeholder, m's slot points to the callee's fresh
                // store, and skip_free on __ref_N suppresses the OpFreeRef
                // that would reclaim the placeholder.
                //
                // The runtime tolerates double-free as a no-op (see
                // database/allocation.rs:103-105: `if store.free { return }`),
                // so leaving __ref_N to be freed by scopes.rs's is_work_ref
                // gate at scope exit is safe in both cases:
                //   - typical adoption: __ref_N's free is a no-op (already
                //     freed via v's free that fires first).
                //   - orphan (P150): __ref_N's free reclaims the placeholder.
                // Mirrors the matching scopes.rs::scan_set change.
                self.generate(value, stack, false);
            }
        } else if matches!(stack.function.tp(v), Type::Vector(_, _)) && *value == Value::Null {
            self.gen_set_first_vector_null(stack, v);
        } else if matches!(stack.function.tp(v), Type::Tuple(_)) && *value == Value::Null {
            self.gen_set_first_tuple_null(stack, v);
        } else if matches!(stack.function.tp(v), Type::Function(_, _, _)) {
            if *value == Value::Null {
                // pre-init a fn-ref slot with 20 null bytes.
                // d_nr = i64::MIN (integer null sentinel) + closure = null DbRef (12B).
                stack.add_op("OpConstInt", self);
                self.code_add(i64::MIN);
                stack.add_op("OpNullRefSentinel", self);
            } else {
                // A5.6-1/A5.6-2: 20-byte fn-ref slot: [d_nr (8B)][closure DbRef (12B)].
                // gen_fn_ref_value ensures every branch (including if-else) reaches the
                // join point with a full 16-byte slot.
                self.gen_fn_ref_value(value, stack);
            }
        } else {
            self.generate(value, stack, false);
        }
    }

    /// First-assignment reference copy from a Call(OpCopyRecord, ...).
    fn gen_set_first_ref_copy(&mut self, stack: &mut Stack, d_nr: u32, value: &Value) {
        // O-B2: if the source is a call to a function with no Reference parameters,
        // the returned store is always fresh — adopt it directly instead of deep copying.
        // This eliminates both the copy overhead and the store leak.
        if let Value::Call(_, args) = value
            && !args.is_empty()
            && let Value::Call(inner_nr, _) = &args[0]
            && matches!(
                stack.data.def(*inner_nr).returned,
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
        {
            let has_ref_params = stack
                .data
                .def(*inner_nr)
                .attributes
                .iter()
                .any(|a| matches!(a.typedef, Type::Reference(_, _) | Type::Enum(_, true, _)));
            if !has_ref_params {
                // Safe: function has no Reference params, cannot return an aliased store.
                // Generate just the inner call — its result DbRef becomes v's value.
                self.generate(&args[0], stack, false);
                return;
            }
        }
        // Fallback: allocate fresh store + deep copy (source store may alias a parameter).
        stack.add_op("OpConvRefFromNull", self);
        stack.add_op("OpDatabase", self);
        self.code_add(size_of::<crate::keys::DbRef>() as u16);
        let tp_nr = stack.data.def(d_nr).known_type;
        self.code_add(tp_nr);
        self.generate(value, stack, false);
    }

    /// First-assignment reference copy from another variable of the same type.
    fn gen_set_first_ref_var_copy(&mut self, stack: &mut Stack, v: u16, src: u16, d_nr: u32) {
        // O-B1: last-use move — if source is only read once (this assignment),
        // transfer the DbRef instead of deep copying. Skip the source's OpFreeRef.
        if stack.function.uses(src) == 1
            && !stack.function.is_argument(src)
            && !stack.function.is_captured(src)
        {
            let src_pos = stack.position - stack.function.stack(src);
            stack.add_op("OpVarRef", self);
            self.code_add(src_pos);
            stack.position += size_of::<crate::keys::DbRef>() as u16;
            stack.function.set_skip_free(src);
            return;
        }
        let tp_nr = stack.data.def(d_nr).known_type;
        stack.add_op("OpConvRefFromNull", self);
        stack.add_op("OpDatabase", self);
        self.code_add(size_of::<crate::keys::DbRef>() as u16);
        self.code_add(tp_nr);
        let copy_nr = stack.data.def_nr("OpCopyRecord");
        let copy_val = Value::Call(
            copy_nr,
            vec![Value::Var(src), Value::Var(v), Value::Int(i32::from(tp_nr))],
        );
        self.generate(&copy_val, stack, false);
    }

    /// First-assignment reference from tuple destructuring — deep copy.
    fn gen_set_first_ref_tuple_copy(
        &mut self,
        stack: &mut Stack,
        v: u16,
        value: &Value,
        d_nr: u32,
    ) {
        let tp_nr = stack.data.def(d_nr).known_type;
        stack.add_op("OpConvRefFromNull", self);
        stack.add_op("OpDatabase", self);
        self.code_add(size_of::<crate::keys::DbRef>() as u16);
        self.code_add(tp_nr);
        let copy_nr = stack.data.def_nr("OpCopyRecord");
        let copy_val = Value::Call(
            copy_nr,
            vec![value.clone(), Value::Var(v), Value::Int(i32::from(tp_nr))],
        );
        self.generate(&copy_val, stack, false);
    }

    /// First-assignment reference from a function call — deep copy to prevent aliasing.
    ///
    /// Sets the `0x8000` "free source" bit on `OpCopyRecord` (issue #120 —
    /// prevents callee store leak), but FIRST locks every ref-typed
    /// argument of the call, then unlocks them after the copy.  The
    /// existing `OpCopyRecord` code at `src/state/io.rs:1001` skips the
    /// source-free when the source store is `locked`, so an early-return
    /// that aliased one of the args (P143:
    /// `return arg.field[i]` inside `for ... in arg.collection { ... }`)
    /// does not free part of the caller's argument.  Args that were
    /// already locked at function entry (const params) get a no-op lock
    /// + a redundant-but-harmless unlock — `n_set_store_lock` doesn't
    ///   touch program-lifetime locked stores (rc >= u32::MAX/2).
    fn gen_set_first_ref_call_copy(&mut self, stack: &mut Stack, v: u16, value: &Value, d_nr: u32) {
        let tp_nr = stack.data.def(d_nr).known_type;
        stack.add_op("OpConvRefFromNull", self);
        stack.add_op("OpDatabase", self);
        self.code_add(size_of::<crate::keys::DbRef>() as u16);
        self.code_add(tp_nr);

        // Collect ref-typed args of the call to bracket with lock/unlock
        // so OpCopyRecord's `0x8000` source-free skips them.
        let ref_args: Vec<u16> = if let Value::Call(fn_nr, args) = value {
            let attrs = stack.data.def(*fn_nr).attributes.clone();
            args.iter()
                .enumerate()
                .filter_map(|(i, arg)| {
                    let tp = attrs.get(i).map(|a| &a.typedef)?;
                    if !matches!(
                        tp,
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    ) {
                        return None;
                    }
                    if let Value::Var(av) = arg {
                        Some(*av)
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let lock_fn = stack.data.def_nr("n_set_store_lock");
        for av in &ref_args {
            let lock = Value::Call(lock_fn, vec![Value::Var(*av), Value::Boolean(true)]);
            self.generate(&lock, stack, false);
        }
        let copy_nr = stack.data.def_nr("OpCopyRecord");
        // High bit = free source store after deep copy.
        // Disabled under WASM: frame yield/resume creates store aliases that rc
        // alone cannot track yet; freeing causes "Allocating a used store" panics.
        //
        // P181: also clear the flag when the callee returns a BORROWED view
        // (its return type carries a `dep` chain naming one of its args).
        // In that case the "source" of the CopyRecord is a slice of the
        // arg's store; freeing it would corrupt the caller.  Return-dep
        // inference already tags these returns correctly — we just need
        // to consult it here.
        #[cfg(not(feature = "wasm"))]
        let tp_with_free = {
            let is_borrowed_view = if let Value::Call(fn_nr, _) = value {
                !stack.data.def(*fn_nr).returned.depend().is_empty()
            } else {
                false
            };
            if is_borrowed_view {
                i32::from(tp_nr)
            } else {
                i32::from(tp_nr) | 0x8000
            }
        };
        #[cfg(feature = "wasm")]
        let tp_with_free = i32::from(tp_nr);
        let copy_val = Value::Call(
            copy_nr,
            vec![value.clone(), Value::Var(v), Value::Int(tp_with_free)],
        );
        self.generate(&copy_val, stack, false);
        for av in &ref_args {
            let unlock = Value::Call(lock_fn, vec![Value::Var(*av), Value::Boolean(false)]);
            self.generate(&unlock, stack, false);
        }
    }

    pub(super) fn clear_stack(&mut self, stack: &mut Stack, loop_nr: u16) {
        let loop_pos = stack.loop_position(loop_nr);
        if stack.position > loop_pos {
            stack.add_op("OpFreeStack", self);
            self.code_add(0u8);
            self.code_add(stack.position - loop_pos);
            stack.position = loop_pos;
        }
    }

    /// Destination-passing for text-producing natives inside `OpAppendText`.
    /// Returns true if the optimisation was applied (caller should return Void).
    fn try_text_dest_pass(&mut self, stack: &mut Stack, op: u32, parameters: &[Value]) -> bool {
        if stack.data.def(op).name != "OpAppendText" || parameters.len() < 2 {
            return false;
        }
        let (Value::Var(dest_var), Value::Call(inner_op, inner_args)) =
            (&parameters[0], &parameters[1])
        else {
            return false;
        };
        let inner_name = stack.data.def(*inner_op).name.clone();
        if !is_text_dest_native(&inner_name) {
            return false;
        }
        let dest_name = inner_name.clone() + "_dest";
        let Some(&lib_nr) = self.library_names.get(&dest_name) else {
            return false;
        };
        let dest_var = *dest_var;
        let inner_op = *inner_op;
        let inner_args = inner_args.clone();
        let inner_attrs: Vec<Type> = stack
            .data
            .def(inner_op)
            .attributes
            .iter()
            .map(|a| a.typedef.clone())
            .collect();
        for arg_val in &inner_args {
            self.generate(arg_val, stack, false);
        }
        stack.add_op("OpCreateStack", self);
        let before_stack = stack.position - size_of::<crate::keys::DbRef>() as u16;
        self.code_add(before_stack - stack.function.stack(dest_var));
        stack.add_op("OpStaticCall", self);
        self.code_add(lib_nr);
        for attr_type in &inner_attrs {
            stack.position -= size(attr_type, &Context::Argument);
        }
        stack.position -= size_of::<crate::keys::DbRef>() as u16;
        true
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn generate_call(
        &mut self,
        stack: &mut Stack,
        op: u32,
        parameters: &[Value],
    ) -> Type {
        let mut tps = Vec::new();
        let mut last = 0;
        let mut was_stack = u16::MAX;
        assert!(
            parameters.len() >= stack.data.def(op).attributes.len(),
            "Too few parameters on {} (got {}, need {})",
            stack.data.def(op).name,
            parameters.len(),
            stack.data.def(op).attributes.len(),
        );
        // S34: suppress OpFreeRef for variables moved to a shared slot by Option A.
        // The outer variable at the same slot emits its own OpFreeRef; emitting a
        // second one would produce a double-free of the same database record.
        if stack.data.def(op).name == "OpFreeRef"
            && let Some(Value::Var(v)) = parameters.first()
            && stack.function.is_skip_free(*v)
        {
            return Type::Void;
        }
        // free the closure DbRef embedded at offset+8 in a 20-byte fn-ref
        // slot.  OpFreeRef normally reads from offset+0, but the fn-ref layout
        // is [d_nr 8B][closure DbRef 12B], so the closure is at var_pos - 8.
        // OpNullRefSentinel produces store_nr=u16::MAX; database.free() is a no-op
        // for that sentinel, so non-capturing lambdas are safe.
        if stack.data.def(op).name == "OpFreeRef"
            && let Some(Value::Var(v)) = parameters.first()
            && matches!(stack.function.tp(*v), Type::Function(_, _, _))
        {
            let var_pos = stack.position - stack.function.stack(*v);
            stack.add_op("OpVarRef", self);
            self.code_add(var_pos - 8);
            stack.add_op("OpFreeRef", self);
            return Type::Void;
        }
        // try destination-passing optimisation for text-producing natives.
        if self.try_text_dest_pass(stack, op, parameters) {
            return Type::Void;
        }
        for (a_nr, a) in stack.data.def(op).attributes.iter().enumerate() {
            if a.mutable {
                let stack_before = stack.position;
                // When a RefVar argument is passed directly to a matching RefVar parameter
                // (e.g. a dispatcher forwarding its text-buffer arg to a variant), emit only
                // OpVarRef to push the raw DbRef — do NOT emit the trailing OpGetStackText /
                // OpGetStackRef that generate_var would normally add.
                if matches!(a.typedef, Type::RefVar(_))
                    && let Value::Var(v) = &parameters[a_nr]
                    && matches!(stack.function.tp(*v), Type::RefVar(_))
                {
                    let var_pos = stack.position - stack.function.stack(*v);
                    stack.add_op("OpVarRef", self);
                    self.code_add(var_pos);
                    tps.push(a.typedef.clone());
                } else {
                    tps.push(self.generate(&parameters[a_nr], stack, false));
                    // When a Value::Null is passed as a typed argument, generate()
                    // pushes 0 bytes.  Emit the correct null sentinel for the
                    // expected type so the stack size matches.
                    if parameters[a_nr] == Value::Null && stack.position == stack_before {
                        self.emit_typed_null(stack, &a.typedef);
                    }
                    // Function args are 16B (4B d_nr + 12B closure DbRef).
                    // A plain fn-ref constant produces only 4B via OpConstInt; pad to 16B.
                    if matches!(a.typedef, Type::Function(_, _, _))
                        && stack.position - stack_before < 16
                    {
                        stack.add_op("OpNullRefSentinel", self);
                    }
                }
                #[cfg(debug_assertions)]
                {
                    let expected = size(&a.typedef, &Context::Argument);
                    let actual = stack.position - stack_before;
                    debug_assert_eq!(
                        actual,
                        expected,
                        "generate_call [{caller}]: mutable arg {a_nr} ({arg_name}: {arg_tp:?}) \
                         expected {expected}B on stack but generate({val:?}) pushed {actual}B — \
                         Value::Null in a typed slot? Missing convert() call in the parser?",
                        caller = stack.data.def(stack.def_nr).name,
                        arg_name = a.name,
                        arg_tp = a.typedef,
                        val = &parameters[a_nr],
                    );
                }
            }
        }
        // push extra Call args beyond the declared parameter count.
        // Only for n_parallel_for — forwards extra context args + n_extra count.
        if stack.data.def(op).name == "n_parallel_for"
            || stack.data.def(op).name == "n_parallel_for_light"
        {
            let n_declared = stack.data.def(op).attributes.len();
            for extra in parameters.iter().skip(n_declared) {
                self.generate(extra, stack, false);
            }
        }
        match &stack.data.def(op).name as &str {
            "OpGetRecord" => {
                was_stack = stack.position;
                self.gather_key(stack, &parameters, 2, &mut tps);
            }
            "OpStart" => {
                was_stack = stack.position + 4 - super::size_ref() as u16;
                self.gather_key(stack, &parameters, 2, &mut tps);
            }
            "OpNext" => {
                was_stack = stack.position;
                self.gather_key(stack, &parameters, 3, &mut tps);
            }
            "OpIterate" => {
                was_stack = stack.position + 8 - super::size_ref() as u16;
                if let Value::Int(parameter_length) = parameters[4] {
                    self.gather_key(stack, &parameters, 4, &mut tps);
                    self.gather_key(stack, &parameters, 5 + parameter_length, &mut tps);
                }
            }
            _ => (),
        }
        if !parameters.is_empty()
            && let Value::Int(n) = parameters[parameters.len() - 1]
        {
            last = n as u16;
        }
        let name = stack.data.def(op).name.clone();
        // CO1.6a: OpCoroutineNext/OpCoroutineExhausted take a gen DbRef from the
        // stack at runtime, but their declarations have only const params.
        // Bypass the operator path and manually handle the stack adjustment.
        if name == "OpCoroutineNext" && parameters.len() >= 2 {
            // CO1.6a: parameters[0]=gen expr, parameters[1]=Int(value_size).
            self.generate(&parameters[0], stack, false); // push DbRef (+12)
            let value_size = if let Value::Int(n) = &parameters[1] {
                *n as u16
            } else {
                4 // fallback: integer
            };
            self.remember_stack(stack.position);
            super::emit_op(stack.data.def(op).op_code, self);
            self.code_add(value_size);
            // Stack: -12 (DbRef consumed) + value_size (yielded value pushed).
            stack.position -= super::size_ref() as u16;
            stack.position += value_size;
            // Return type is the yield type — inferred from value_size for now.
            return match value_size {
                1 => Type::Boolean,
                8 => crate::data::I64.clone(),
                _ => I32.clone(),
            };
        }
        if name == "OpCoroutineExhausted" && !parameters.is_empty() {
            // parameters[0] is the gen expression — generate it (pushes DbRef, +12).
            self.generate(&parameters[0], stack, false);
            self.remember_stack(stack.position);
            super::emit_op(stack.data.def(op).op_code, self);
            // Stack: -12 (DbRef consumed) + 1 (bool pushed).
            stack.position -= super::size_ref() as u16;
            stack.position += 1;
            return Type::Boolean;
        }
        // resolve library index — prefer #native symbol, fall back to def name.
        // P145: only use the library_names fallback for functions without a user-
        // defined body.  A user function whose `n_<name>` collides with a native
        // stdlib name (e.g. user `to_json` vs native `n_to_json` for JsonValue)
        // must go through OpCall, not OpStaticCall.
        let native_sym = stack.data.def(op).native.clone();
        let has_user_body = stack.data.def(op).code != Value::Null;
        let lib_lookup: &str = if !native_sym.is_empty() {
            &native_sym
        } else if has_user_body {
            // User function with a body — never route through native dispatch,
            // even if the name happens to match a library_names entry.
            ""
        } else {
            &name
        };
        let lib_nr = if lib_lookup.is_empty() {
            None
        } else {
            self.library_names.get(lib_lookup).copied()
        };
        // P160: OpCreateStack with a non-Var expression argument (e.g.
        // OpGetVector result).  The runtime reads a u16 offset from the code
        // stream, but add_const writes nothing for Type::Reference args.
        // Handle here: generate the expression (pushes a 12-byte DbRef),
        // then emit OpCreateStack with the offset pointing at the just-
        // pushed result.
        if name == "OpCreateStack"
            && !parameters.is_empty()
            && !matches!(&parameters[0], Value::Var(_))
        {
            self.generate(&parameters[0], stack, false);
            stack.add_op("OpCreateStack", self);
            self.code_add(size_of::<crate::keys::DbRef>() as u16);
            return stack.data.def(op).returned.clone();
        }
        if stack.data.def(op).is_operator() {
            // B7: OpAppendCharacter on a RefVar(Text) target must use
            // OpAppendStackCharacter — same pattern as OpAppendText →
            // OpAppendStackText in set_var.  Without this, the append
            // writes to the raw RefVar slot instead of dereferencing it.
            let actual_op = if name == "OpAppendCharacter"
                && let Some(Value::Var(v)) = parameters.first()
                && matches!(stack.function.tp(*v), Type::RefVar(_))
            {
                stack.data.def_nr("OpAppendStackCharacter")
            } else {
                op
            };
            let before_stack = stack.position;
            self.remember_stack(stack.position);
            let code = self.code_pos;
            super::emit_op(stack.data.def(actual_op).op_code, self);
            stack.operator(actual_op);
            if was_stack != u16::MAX {
                stack.position = was_stack;
            }
            for (a_nr, a) in stack.data.def(op).attributes.iter().enumerate() {
                if a.mutable {
                    continue;
                }
                // OpIterate: from_key is at parameters[4], but till_key is at
                // parameters[5 + from_count] because the from-key values occupy
                // parameters[5..5+from_count], pushing till_key_count further out.
                let param_idx = if name == "OpIterate"
                    && a_nr == 5
                    && let Value::Int(from_count) = parameters[4]
                {
                    (5 + from_count) as usize
                } else {
                    a_nr
                };
                self.add_const(&a.typedef, &parameters[param_idx], stack, before_stack);
            }
            self.op_type(op, &tps, last, code, stack)
        } else if let Some(lib_idx) = lib_nr {
            stack.add_op("OpStaticCall", self);
            self.code_add(lib_idx);
            for a in &stack.data.def(op).attributes {
                stack.position -= size(&a.typedef, &Context::Argument);
            }
            // also subtract the extra args pushed beyond declared params.
            if stack.data.def(op).name == "n_parallel_for"
                || stack.data.def(op).name == "n_parallel_for_light"
            {
                let n_declared = stack.data.def(op).attributes.len();
                for extra in parameters.iter().skip(n_declared) {
                    // Extra args are always integer (8 bytes post-2c).
                    let _ = extra;
                    stack.position -= 8;
                }
            }
            // add the result to the stack
            stack.position += size(&stack.data.def(op).returned, &Context::Argument);
            stack.data.def(op).returned.clone()
        } else {
            self.calls.entry(op).or_default().push(self.code_pos);
            // CO1.3c: emit OpCoroutineCreate for generator function calls.
            let is_generator = matches!(stack.data.def(op).returned, Type::Iterator(_, _));
            if is_generator {
                stack.add_op("OpCoroutineCreate", self);
            } else {
                stack.add_op("OpCall", self);
            }
            self.code_add(i64::from(op)); // d_nr: i64 (stdlib `const i32` widens post-2c)
            let args_size: u16 = stack
                .data
                .def(op)
                .attributes
                .iter()
                .map(|a| size(&a.typedef, &Context::Argument))
                .sum();
            self.code_add(args_size);
            self.code_add(i64::from(stack.data.def(op).code_position));
            // remove the arguments that are already on the stack
            for a in &stack.data.def(op).attributes {
                stack.position -= size(&a.typedef, &Context::Argument);
            }
            // add the result to the stack
            stack.position += size(&stack.data.def(op).returned, &Context::Argument);
            stack.data.def(op).returned.clone()
        }
    }

    pub(super) fn gather_key(
        &mut self,
        stack: &mut Stack,
        parameters: &&[Value],
        from: i32,
        tps: &mut Vec<Type>,
    ) {
        let no_keys = if let Value::Int(v) = &parameters[from as usize] {
            *v
        } else {
            0
        };
        for k in 0..no_keys {
            tps.push(self.generate(&parameters[(no_keys + from - k) as usize], stack, false));
        }
    }

    pub(super) fn op_type(
        &mut self,
        op: u32,
        tps: &[Type],
        last: u16,
        code: u32,
        stack: &mut Stack,
    ) -> Type {
        match &stack.data.def(op).name as &str {
            "OpDatabase" | "OpAppend" | "OpConvEnumFromNull" | "OpCastEnumFromInt"
            | "OpCastEnumFromText" | "OpGetField" => {
                self.types.insert(code, last);
            }
            "OpGetVector" | "OpVectorRef" | "OpInsertVector" | "OpAppendVector" => {
                if let Type::Vector(v, _) = &tps[0] {
                    self.types
                        .insert(code, stack.data.def(stack.data.type_def_nr(v)).known_type);
                    return *v.clone();
                }
            }
            "OpGetHash" => {
                if let Type::Hash(v, _, link) = &tps[0] {
                    return Type::Reference(*v, link.clone());
                }
            }
            "OpGetIndex" => {
                if let Type::Index(v, _, link) = &tps[0] {
                    return Type::Reference(*v, link.clone());
                }
            }
            "OpGetSpacial" => {
                if let Type::Spacial(v, _, link) = &tps[0] {
                    return Type::Reference(*v, link.clone());
                }
            }
            "OpVarEnum" => {
                self.insert_types(tps[0].clone(), code, stack);
            }
            _ => (),
        }
        stack.data.def(op).returned.clone()
    }

    pub(super) fn insert_types(&mut self, tp: Type, code: u32, stack: &Stack) -> Type {
        match tp {
            Type::Enum(t, _, _) => {
                self.types.insert(code, stack.data.def(t).known_type);
            }
            Type::Reference(t, _) if t < u32::from(u16::MAX) => {
                self.types.insert(code, stack.data.def(t).known_type);
            }
            _ => (),
        }
        tp
    }

    /// Emit bytecode for a call through a fn-ref variable.
    ///
    /// Use when the callee is `Value::CallRef(v_nr, args)` — the fn-ref is stored as an
    /// i32 `d_nr` in a local variable; arguments are already type-checked by the parser.
    pub(super) fn generate_call_ref(
        &mut self,
        stack: &mut Stack,
        v_nr: u16,
        args: &[Value],
    ) -> Type {
        let Type::Function(param_types, ret_type, _) = stack.function.tp(v_nr).clone() else {
            panic!("generate_call_ref: variable is not Type::Function");
        };
        let ret_type = *ret_type;

        // Generate all args: visible params, then work-buffer blocks (12B DbRefs each),
        // then closure arg (12B DbRef).  Blocks produce the correct type/size automatically.
        for arg in args {
            self.generate(arg, stack, false);
        }

        // fn-ref variable is below all pushed arguments.
        let fn_var_dist = stack.position - stack.function.stack(v_nr);
        // declared: visible param sizes; extra: work-buf + closure (all 12-byte DbRefs).
        let declared_size: u16 = param_types
            .iter()
            .map(|t| size(t, &Context::Argument))
            .sum();
        let extra = if args.len() > param_types.len() {
            (args.len() - param_types.len()) as u16 * super::size_ref() as u16
        } else {
            0
        };
        let total_arg_size = declared_size + extra;
        stack.add_op("OpCallRef", self);
        self.code_add(fn_var_dist);
        self.code_add(total_arg_size);
        stack.position -= total_arg_size;
        stack.position += size(&ret_type, &Context::Argument);
        ret_type
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn generate_var(&mut self, stack: &mut Stack, variable: u16) -> Type {
        assert!(
            stack.function.stack(variable) <= stack.position,
            "Incorrect var {}[{}] versus {} on {}",
            stack.function.name(variable),
            stack.function.stack(variable),
            stack.position,
            stack.data.def(stack.def_nr).name
        );
        let var_pos = stack.position - stack.function.stack(variable);
        let argument = stack.function.is_argument(variable);
        let code = self.code_pos;
        self.vars.insert(code, variable);
        match stack.function.tp(variable) {
            Type::Integer(_, _, _) => stack.add_op("OpVarInt", self),
            Type::Function(_, _, _) => {
                stack.add_op("OpVarFnRef", self);
                // Post-2c fn-ref slot is 20 bytes, but OpVarFnRef's stdlib
                // signature returns `text` (16 B Str).  Add the 4-byte
                // discrepancy to the compile-time stack tracker.
                stack.position += 4;
            }
            Type::Character => stack.add_op("OpVarCharacter", self),
            Type::RefVar(_) => stack.add_op("OpVarRef", self),
            Type::Enum(_, false, _) => stack.add_op("OpVarEnum", self),
            Type::Boolean => stack.add_op("OpVarBool", self),
            Type::Single => stack.add_op("OpVarSingle", self),
            Type::Float => stack.add_op("OpVarFloat", self),
            Type::Text(_) => {
                stack.add_op(if argument { "OpArgText" } else { "OpVarText" }, self);
            }
            Type::Vector(tp, _) => {
                let typedef: &Type = tp;
                let known = if matches!(typedef, Type::Unknown(_)) {
                    u16::MAX
                } else if matches!(typedef, Type::Text(_)) {
                    self.database.vector(5)
                } else {
                    let name = typedef.name(stack.data);
                    let mut tp_nr = self.database.name(&name);
                    if tp_nr == u16::MAX {
                        tp_nr = self.database.db_type(typedef, stack.data);
                    }
                    self.database.vector(tp_nr)
                };
                if known != u16::MAX {
                    self.types.insert(self.code_pos, known);
                }
                stack.add_op("OpVarVector", self);
            }
            Type::Reference(c, _) | Type::Enum(c, true, _) => {
                self.types
                    .insert(self.code_pos, stack.data.def(*c).known_type);
                stack.add_op("OpVarRef", self);
            }
            // CO1.3c: iterator variables are DbRef-sized (coroutine frame reference).
            Type::Iterator(_, _) => {
                stack.add_op("OpVarRef", self);
            }
            Type::Tuple(elems) => {
                // T1.4: read whole tuple by reading each element.
                let elems = elems.clone();
                let tuple_base = stack.function.stack(variable);
                let offsets = crate::data::element_offsets(&elems);
                for (i, elem_tp) in elems.iter().enumerate() {
                    let elem_pos = stack.position - (tuple_base + offsets[i] as u16);
                    match elem_tp {
                        Type::Integer(_, _, _) | Type::Function(_, _, _) => {
                            stack.add_op("OpVarInt", self);
                        }
                        Type::Boolean => stack.add_op("OpVarBool", self),
                        Type::Float => stack.add_op("OpVarFloat", self),
                        Type::Single => stack.add_op("OpVarSingle", self),
                        Type::Character => stack.add_op("OpVarCharacter", self),
                        Type::Enum(_, false, _) => stack.add_op("OpVarEnum", self),
                        Type::Text(_) => stack.add_op("OpArgText", self),
                        Type::Reference(c, _) | Type::Enum(c, true, _) => {
                            self.types
                                .insert(self.code_pos, stack.data.def(*c).known_type);
                            stack.add_op("OpVarRef", self);
                        }
                        _ => panic!("Tuple var: unsupported element type {elem_tp:?}"),
                    }
                    self.code_add(elem_pos);
                    // Note: add_op already adjusts stack.position for the pushed value.
                }
                return self.insert_types(stack.function.tp(variable).clone(), code, stack);
            }
            _ => panic!(
                "Unknown var '{}' type {} at {}",
                stack.function.name(variable),
                stack.function.tp(variable).name(stack.data),
                stack.data.def(stack.def_nr).position
            ),
        }
        self.code_add(var_pos);
        if let Type::RefVar(tp) = stack.function.tp(variable) {
            let txt = matches!(**tp, Type::Text(_));
            match &**tp {
                Type::Integer(_, _, _) => stack.add_op("OpGetInt", self),
                Type::Character => stack.add_op("OpGetCharacter", self),
                Type::Single => stack.add_op("OpGetSingle", self),
                Type::Float => stack.add_op("OpGetFloat", self),
                Type::Enum(_, false, _) => stack.add_op("OpGetByte", self),
                Type::Text(_) => stack.add_op("OpGetStackText", self),
                Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, true, _) => {
                    stack.add_op("OpGetStackRef", self);
                }
                _ => panic!("Unknown referenced variable type: {tp}"),
            }
            if !txt {
                self.code_add(0u16);
            }
        }
        self.insert_types(stack.function.tp(variable).clone(), code, stack)
    }

    pub(super) fn generate_block(&mut self, stack: &mut Stack, block: &Block, top: bool) -> Type {
        if block.operators.is_empty() {
            return Type::Void;
        }
        let to = stack.position;

        // Pre-claim small-variable (zone1) frame so large-type init opcodes see exact TOS.
        if block.var_size > 0 {
            stack.add_op("OpReserveFrame", self);
            self.code_add(block.var_size);
            stack.position += block.var_size;
        }

        let mut tp = Type::Void;
        let mut return_expr = 0;
        let mut has_return = false;
        for v in &block.operators {
            let s_pos = self.stack_pos;
            if let Value::Return(expr) = v {
                has_return = true;
                if return_expr == 0 {
                    return_expr = s_pos;
                    self.generate(expr, stack, false);
                }
                self.add_return(stack, return_expr);
                return_expr = 0;
                tp = Type::Void;
            } else {
                // Preserve return_expr across cleanup ops (FreeRef/FreeText)
                // that don't produce a return value. These are inserted by
                // scope analysis between the tail expression and Return(Null).
                let is_cleanup = matches!(v, Value::Call(d, _)
                    if stack.data.def(*d).name == "OpFreeRef"
                    || stack.data.def(*d).name == "OpFreeText");
                if !is_cleanup {
                    has_return = false;
                    return_expr = 0;
                }
                tp = self.generate(v, stack, false);
            }
            if self.stack_pos > s_pos && !matches!(v, Value::Set(_, _)) {
                // Normal expressions do not claim stack space (because of Value::Drop).
                // So, if there is data left, it should be a return expression.
                return_expr = s_pos;
            }
        }
        if top {
            if !has_return {
                self.add_return(
                    stack,
                    if return_expr > 0 {
                        return_expr
                    } else {
                        self.code_pos
                    },
                );
            }
        } else {
            let size = size(&block.result, &Context::Argument);
            let after = to + size;
            if stack.position > after {
                stack.add_op("OpFreeStack", self);
                self.code_add(size as u8);
                self.code_add(stack.position - to);
            } else if matches!(&block.result, Type::Function(_, _, _)) && stack.position < after {
                // a fn-ref block result is 16 bytes ([d_nr 4B][closure DbRef 12B]).
                // If the block only pushed 4 bytes (d_nr via OpConstInt), pad to 16 with
                // OpNullRefSentinel so both branches of an if-else reach the join point with
                // the same stack delta.
                stack.add_op("OpNullRefSentinel", self);
            }
            stack.position = after;
        }
        tp
    }

    pub(super) fn add_return(&mut self, stack: &mut Stack, code: u32) {
        let return_type = &stack.data.def(stack.def_nr).returned;
        // CO1.3c: generator functions use OpCoroutineReturn.
        if matches!(return_type, Type::Iterator(_, _)) {
            let yield_size = if let Type::Iterator(inner, _) = return_type {
                size(inner, &Context::Argument)
            } else {
                0
            };
            stack.add_op("OpCoroutineReturn", self);
            self.code_add(yield_size);
        } else {
            stack.add_op("OpReturn", self);
            self.code_add(self.arguments);
            self.code_add(size(return_type, &Context::Argument) as u8);
            self.code_add(stack.position);
            if return_type != &Type::Void {
                self.types.insert(code, self.known_type(return_type, stack));
            }
        }
    }

    pub(super) fn known_type(&self, tp: &Type, stack: &Stack) -> u16 {
        if let Some(c) = tp.heap_def_nr() {
            stack.data.def(c).known_type
        } else {
            self.database.name(&tp.name(stack.data))
        }
    }

    pub(super) fn add_const(&mut self, tp: &Type, p: &Value, stack: &Stack, before_stack: u16) {
        match tp {
            Type::Integer(0, 255, _) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as u8);
                }
            }
            Type::Integer(-128, 127, _) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as i8);
                }
            }
            Type::Integer(0, 65535, _) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as u16);
                } else if let Value::Var(v) = p {
                    let r = stack.function.stack(*v);
                    self.code_add(before_stack - r);
                }
            }
            Type::Integer(-32768, 32767, _) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as i16);
                }
            }
            Type::Integer(_, _, _) => {
                if let Value::Int(nr) = p {
                    self.code_add(i64::from(*nr));
                } else if let Value::Long(val) = p {
                    self.code_add(*val);
                }
            }
            Type::Enum(_, _, _) => {
                if let Value::Enum(nr, _) = p {
                    self.code_add(*nr);
                }
            }
            Type::Boolean => {
                if let Value::Boolean(v) = p {
                    self.code_add(u8::from(*v));
                }
            }
            Type::Text(_) => {
                if let Value::Text(s) = p {
                    self.code_add_str(s);
                }
            }
            Type::Float => {
                if let Value::Float(val) = p {
                    self.code_add(*val);
                }
            }
            Type::Single => {
                if let Value::Single(val) = p {
                    self.code_add(*val);
                }
            }
            Type::Keys => {
                if let Value::Keys(keys) = p {
                    self.code_add(keys.len() as u8);
                    for k in keys {
                        self.code_add(k.type_nr);
                        self.code_add(k.position);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn set_var(&mut self, stack: &mut Stack, var: u16, value: &Value) {
        if let Type::RefVar(tp) = stack.function.tp(var).clone() {
            if matches!(*tp, Type::Text(_)) {
                if value == &Value::Text(String::new()) {
                    return;
                }
                // always clear RefVar(Text) before appending — prevents
                // text accumulation across reassignments in text-returning functions.
                {
                    let var_pos = stack.position - stack.function.stack(var);
                    stack.add_op("OpClearStackText", self);
                    self.code_add(var_pos);
                }
                self.generate(value, stack, false);
                let var_pos = stack.position - stack.function.stack(var);
                stack.add_op("OpAppendStackText", self);
                self.code_add(var_pos);
                return;
            }
            let var_pos = stack.position - stack.function.stack(var);
            stack.add_op("OpVarRef", self);
            self.code_add(var_pos);
            self.generate(value, stack, false);
            match *tp {
                Type::Integer(_, _, _) => stack.add_op("OpSetInt", self),
                Type::Character => stack.add_op("OpSetCharacter", self),
                Type::Single => stack.add_op("OpSetSingle", self),
                Type::Float => stack.add_op("OpSetFloat", self),
                Type::Enum(_, false, _) => stack.add_op("OpSetByte", self),
                Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, true, _) => {
                    stack.add_op("OpSetStackRef", self);
                }
                _ => panic!("Unknown reference variable type"),
            }
            self.code_add(0u16);
            return;
        }
        // destination-passing — avoid scratch buffer for text-returning natives.
        if matches!(stack.function.tp(var), Type::Text(_))
            && let Value::Call(op, args) = value
        {
            let name = stack.data.def(*op).name.clone();
            if is_text_dest_native(&name) {
                let dest_name = name.clone() + "_dest";
                if let Some(&lib_nr) = self.library_names.get(&dest_name) {
                    self.gen_text_dest_call(stack, var, *op, args, lib_nr);
                    return;
                }
            }
        }
        self.generate(value, stack, false);
        let var_pos = stack.position - stack.function.stack(var);
        match stack.function.tp(var) {
            Type::Integer(_, _, _) => stack.add_op("OpPutInt", self),
            Type::Function(_, _, _) => {
                stack.add_op("OpPutFnRef", self);
                // Post-2c fn-ref slot is 20 bytes, but OpPutFnRef's stdlib
                // signature pops `text` (16 B Str).  Subtract the 4-byte
                // discrepancy from the compile-time stack tracker.
                stack.position -= 4;
            }
            Type::Character => stack.add_op("OpPutCharacter", self),
            Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
            Type::Boolean => stack.add_op("OpPutBool", self),
            Type::Single => stack.add_op("OpPutSingle", self),
            Type::Float => stack.add_op("OpPutFloat", self),
            Type::Text(_) => {
                if value == &Value::Text(String::new()) {
                    return;
                }
                stack.add_op("OpAppendText", self);
            }
            Type::Vector(_, _)
            | Type::Reference(_, _)
            | Type::Enum(_, true, _)
            | Type::Iterator(_, _) => {
                stack.add_op("OpPutRef", self);
            }
            Type::Tuple(elems) => {
                // T1.4: store each element from the stack into the variable.
                // Elements are on the stack in order; emit OpPut* for each in
                // reverse order (last element is at top of stack).
                let elems = elems.clone();
                let offsets = crate::data::element_offsets(&elems);
                let tuple_var_base = stack.function.stack(var);
                for i in (0..elems.len()).rev() {
                    let elem_abs = tuple_var_base + offsets[i] as u16;
                    // After popping previous elements, adjust position.
                    let pos = stack.position - elem_abs;
                    match &elems[i] {
                        Type::Integer(_, _, _) | Type::Function(_, _, _) => {
                            stack.add_op("OpPutInt", self);
                        }
                        Type::Boolean => stack.add_op("OpPutBool", self),
                        Type::Float => stack.add_op("OpPutFloat", self),
                        Type::Single => stack.add_op("OpPutSingle", self),
                        Type::Character => stack.add_op("OpPutCharacter", self),
                        Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
                        Type::Text(_) => stack.add_op("OpPutText", self),
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => {
                            stack.add_op("OpPutRef", self);
                        }
                        _ => panic!("Tuple set: unsupported element type {:?}", elems[i]),
                    }
                    self.code_add(pos);
                    // Note: add_op already adjusts stack.position for the popped element.
                }
                return;
            }
            _ => panic!(
                "Unknown var {} type {} at {}",
                stack.function.name(var),
                stack.function.tp(var).name(stack.data),
                stack.data.def(stack.def_nr).position
            ),
        }
        self.code_add(var_pos);
    }

    /// Emit a destination-passing call for a text-returning native function.
    ///
    /// Instead of: evaluate call → Str on stack → OpAppendText(var)
    /// Emits:      args → OpCreateStack(var) → `OpStaticCall`  (native writes to var directly)
    fn gen_text_dest_call(
        &mut self,
        stack: &mut Stack,
        var: u16,
        op: u32,
        args: &[Value],
        lib_nr: u16,
    ) {
        let attr_types: Vec<Type> = stack
            .data
            .def(op)
            .attributes
            .iter()
            .map(|a| a.typedef.clone())
            .collect();
        for arg_val in args {
            self.generate(arg_val, stack, false);
        }
        stack.add_op("OpCreateStack", self);
        let before_stack = stack.position - size_of::<crate::keys::DbRef>() as u16;
        self.code_add(before_stack - stack.function.stack(var));
        stack.add_op("OpStaticCall", self);
        self.code_add(lib_nr);
        for attr_type in &attr_types {
            stack.position -= size(attr_type, &Context::Argument);
        }
        stack.position -= size_of::<crate::keys::DbRef>() as u16;
    }
}

/// Check if a Value is a divergent expression (return/break/continue)
/// that never produces a value at the join point.
fn is_divergent(val: &Value) -> bool {
    match val {
        Value::Return(_) | Value::Break(_) | Value::BreakWith(_, _) | Value::Continue(_) => true,
        // scopes.rs wraps `return` in `Insert([free_ops..., Return(...)])` so the
        // raw-Return check misses it. Walk the last op of Insert/Block to recover
        // divergence for these wrappers.
        Value::Insert(ops) => ops.last().is_some_and(is_divergent),
        Value::Block(bl) => bl.operators.last().is_some_and(is_divergent),
        _ => false,
    }
}

/// Recursively checks whether `value` contains a direct `Var(v)` reference.
/// Used in debug builds to detect first-assignment self-reference bugs.
#[cfg(debug_assertions)]
fn ir_contains_var(value: &Value, v: u16) -> bool {
    match value {
        Value::Var(n) => *n == v,
        Value::Call(_, args) => args.iter().any(|a| ir_contains_var(a, v)),
        Value::CallRef(_, args) => args.iter().any(|a| ir_contains_var(a, v)),
        Value::Set(_, inner) | Value::Return(inner) | Value::Drop(inner) => {
            ir_contains_var(inner, v)
        }
        Value::If(cond, then, els) => {
            ir_contains_var(cond, v) || ir_contains_var(then, v) || ir_contains_var(els, v)
        }
        Value::Block(b) | Value::Loop(b) => b.operators.iter().any(|op| ir_contains_var(op, v)),
        Value::Insert(items) => items.iter().any(|i| ir_contains_var(i, v)),
        Value::Iter(_, create, next, extra) => {
            ir_contains_var(create, v) || ir_contains_var(next, v) || ir_contains_var(extra, v)
        }
        _ => false,
    }
}

/// Recursively prints a `Value` IR tree to stderr in a loft-like syntax.
///
/// Gated by the `LOFT_IR` environment variable (set to a function-name filter
/// or `*` for all); only compiled in debug builds to keep release binaries
/// clean.  Produces a lot of output for large functions, so the filter is
/// important.
#[cfg(debug_assertions)]
#[allow(clippy::too_many_lines)]
fn print_ir(value: &Value, data: &crate::data::Data, vars: &Function, depth: usize) {
    let pad = "  ".repeat(depth);
    match value {
        Value::Null => eprint!("null"),
        Value::Int(n) => eprint!("{n}"),
        Value::Long(n) => eprint!("{n}L"),
        Value::Float(f) => eprint!("{f}"),
        Value::Single(f) => eprint!("{f}f"),
        Value::Boolean(b) => eprint!("{b}"),
        Value::Enum(v, tp) => eprint!("enum({v},tp={tp})"),
        Value::Text(s) => eprint!("{s:?}"),
        Value::Line(_) => {} // source-line markers: skip
        Value::Var(n) => eprint!("{}", vars.name(*n)),
        Value::Break(n) => eprint!("break({n})"),
        Value::BreakWith(n, inner) => {
            eprint!("break({n}) ");
            print_ir(inner, data, vars, depth);
        }
        Value::Continue(n) => eprint!("continue({n})"),
        Value::Keys(keys) => eprint!("keys({keys:?})"),
        Value::Set(v, inner) => {
            eprint!("{} = ", vars.name(*v));
            print_ir(inner, data, vars, depth);
        }
        Value::Return(inner) => {
            eprint!("return ");
            print_ir(inner, data, vars, depth);
        }
        Value::Drop(inner) => {
            eprint!("drop(");
            print_ir(inner, data, vars, depth);
            eprint!(")");
        }
        Value::Call(d, args) => {
            eprint!("{}(", data.def(*d).name);
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    eprint!(", ");
                }
                print_ir(a, data, vars, depth);
            }
            eprint!(")");
        }
        Value::CallRef(v, args) => {
            eprint!("fn_ref[{}](", vars.name(*v));
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    eprint!(", ");
                }
                print_ir(a, data, vars, depth);
            }
            eprint!(")");
        }
        Value::Block(b) => {
            eprintln!("{{  // {}", b.name);
            for op in &b.operators {
                eprint!("{pad}  ");
                print_ir(op, data, vars, depth + 1);
                eprintln!();
            }
            eprint!("{pad}}}");
        }
        Value::Loop(b) => {
            eprint!("loop ");
            print_ir(&Value::Block(b.clone()), data, vars, depth);
        }
        Value::Insert(items) => {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    eprint!("{pad}  ");
                }
                print_ir(item, data, vars, depth);
                if i + 1 < items.len() {
                    eprintln!();
                }
            }
        }
        Value::If(cond, then, els) => {
            eprint!("if ");
            print_ir(cond, data, vars, depth);
            eprint!(" ");
            print_ir(then, data, vars, depth);
            if **els != Value::Null {
                eprint!(" else ");
                print_ir(els, data, vars, depth);
            }
        }
        Value::Iter(v, create, next, extra) => {
            eprint!("for {} in ", vars.name(*v));
            print_ir(create, data, vars, depth);
            if **extra != Value::Null {
                eprint!(", extra=");
                print_ir(extra, data, vars, depth);
            }
            // `next` is the advance expression: omit for brevity
            let _ = next;
        }
        Value::Tuple(elems) => {
            eprint!("(");
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    eprint!(", ");
                }
                print_ir(e, data, vars, depth);
            }
            eprint!(")");
        }
        Value::TupleGet(var, idx) => {
            eprint!("{}.{idx}", vars.name(*var));
        }
        Value::TuplePut(var, idx, val) => {
            eprint!("{}.{idx} = ", vars.name(*var));
            print_ir(val, data, vars, depth);
        }
        Value::Yield(inner) => {
            eprint!("yield ");
            print_ir(inner, data, vars, depth);
        }
        Value::FnRef(d_nr, clos_var, _) => {
            eprint!("FnRef({d_nr}, clos={})", vars.name(*clos_var));
        }
        Value::Parallel(arms) => {
            eprintln!("parallel {{");
            for arm in arms {
                eprint!("{pad}  ");
                print_ir(arm, data, vars, depth + 1);
                eprintln!(";");
            }
            eprint!("{pad}}}");
        }
    }
}
