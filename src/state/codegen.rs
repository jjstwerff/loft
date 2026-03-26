// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::State;
use crate::data::{Block, Context, Data, I32, Type, Value};
use crate::stack::Stack;
#[cfg(debug_assertions)]
use crate::variables::Function;
use crate::variables::size;
use std::collections::HashSet;
use std::sync::Arc;

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
            let start = self.code_pos;
            self.add_return(&mut stack, start);
            data.definitions[def_nr as usize].code_position = start;
            data.definitions[def_nr as usize].code_length = self.code_pos - start;
            return;
        }
        let is_empty_stub =
            matches!(&stack.data.def(def_nr).code, Value::Block(bl) if bl.operators.is_empty());
        for a in 0..stack.data.def(def_nr).attributes.len() as u16 {
            let n = &stack.data.def(def_nr).attributes[a as usize].name;
            let v = stack.function.var(n);
            if v != u16::MAX {
                stack.function.set_stack_pos(v, stack.position);
                stack.position += size(stack.function.tp(v), &Context::Argument);
            }
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
        // Only compiled in debug builds; produces one block per matching function.
        #[cfg(debug_assertions)]
        if let Ok(filter) = std::env::var("LOFT_IR") {
            let fn_name = stack.data.def(def_nr).name.as_str();
            let want_all = filter.is_empty() || filter == "*";
            let matches = want_all || filter == fn_name || fn_name.contains(&*filter);
            if matches && logging {
                eprintln!("=== IR: {fn_name} ===");
                print_ir(&stack.data.def(def_nr).code, stack.data, &stack.function, 0);
                eprintln!();
                eprintln!("===");
            }
        }
        self.source = stack.data.def(def_nr).source;
        self.generate(&stack.data.def(def_nr).code, &mut stack, true);
        let mut stack_pos = Vec::new();
        for v_nr in 0..stack.function.next_var() {
            stack_pos.push(stack.function.stack(v_nr));
        }
        data.definitions[def_nr as usize].code_position = start;
        data.definitions[def_nr as usize].code_length = self.code_pos - start;
        if let Some(v) = self.calls.get(&def_nr) {
            let old = self.code_pos;
            for pos in v.clone() {
                // skip opcode(1) + d_nr(4) + args_size(2) to reach the i32 target
                self.code_pos = pos + 7;
                self.code_add(start as i32);
            }
            self.code_pos = old;
        }
        for (v_nr, pos) in stack_pos.into_iter().enumerate() {
            data.definitions[def_nr as usize]
                .variables
                .set_stack(v_nr as u16, pos);
        }
        #[cfg(debug_assertions)]
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
                self.code_add(*value);
                I32.clone()
            }
            Value::Enum(value, tp) => {
                self.types.insert(self.code_pos, *tp);
                stack.add_op("OpConstEnum", self);
                self.code_add(*value);
                Type::Enum(0, false, Vec::new())
            }
            Value::Long(value) => {
                stack.add_op("OpConstLong", self);
                self.code_add(*value);
                Type::Long
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
            Value::TupleGet(var_nr, elem_idx) => {
                // T1.4: read element elem_idx from tuple variable var_nr.
                let tuple_tp = stack.function.tp(*var_nr).clone();
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
                    Type::Integer(_, _) | Type::Function(_, _) => {
                        stack.add_op("OpVarInt", self);
                    }
                    Type::Boolean => stack.add_op("OpVarBool", self),
                    Type::Long => stack.add_op("OpVarLong", self),
                    Type::Float => stack.add_op("OpVarFloat", self),
                    Type::Single => stack.add_op("OpVarSingle", self),
                    Type::Character => stack.add_op("OpVarCharacter", self),
                    Type::Enum(_, false, _) => stack.add_op("OpVarEnum", self),
                    Type::Text(_) => stack.add_op("OpVarText", self),
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
                Type::Void
            }
        }
    }

    pub(super) fn gen_text(&mut self, value: &str, stack: &mut Stack) -> Type {
        if value.len() < 256 {
            stack.add_op("OpConstText", self);
            self.code_add_str(value);
        } else {
            let tc = Arc::make_mut(&mut self.text_code);
            debug_assert!(
                i32::try_from(tc.len()).is_ok(),
                "text_code offset overflow: {}",
                tc.len()
            );
            let start = tc.len() as i32;
            tc.extend_from_slice(value.as_bytes());
            stack.add_op("OpConstLongText", self);
            self.code_add(start);
            debug_assert!(
                i32::try_from(value.len()).is_ok(),
                "long-text length overflow: {}",
                value.len()
            );
            self.code_add(value.len() as i32);
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
        if *f_val == Value::Null {
            self.code_put(code_step, (self.code_pos - true_pos) as i16); // actual step
        } else {
            stack.add_op("OpGotoWord", self);
            let end = self.code_pos;
            self.code_add(0i16); // temp end
            let false_pos = self.code_pos;
            self.code_put(code_step, (self.code_pos - true_pos) as i16); // actual step
            stack.position = stack_pos;
            self.generate(f_val, stack, false);
            self.code_put(end, (self.code_pos - false_pos) as i16); // actual end
        }
        tp
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
            stack.add_op("OpCreateStack", self);
            let dep_pos = stack.function.stack(dep[0]);
            let before_stack = stack.position - size_of::<crate::keys::DbRef>() as u16;
            self.code_add(before_stack - dep_pos);
        }
    }

    pub(super) fn gen_set_first_vector_null(&mut self, stack: &mut Stack, v: u16) {
        if let Type::Vector(elm_tp, dep) = stack.function.tp(v).clone() {
            if dep.is_empty() {
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
                self.code_add(0);
                stack.add_op("OpSetInt", self);
                self.code_add(4u16);
                stack.add_op("OpCreateStack", self);
                self.code_add(size_of::<crate::keys::DbRef>() as u16);
                stack.add_op("OpConstInt", self);
                self.code_add(12);
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

    /// Adjust the slot position for a first-assignment variable.
    /// Case 1: pre-assigned above TOS → move down. Case 2: large type below TOS →
    /// override only if no child-scope overlap (A13 guard).
    fn adjust_first_assignment_slot(stack: &mut Stack, v: u16, pos: u16) {
        if pos > stack.position {
            stack.function.set_stack_pos(v, stack.position);
        } else if pos < stack.position
            && matches!(
                stack.function.tp(v),
                Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, true, _)
            )
        {
            let v_size = size(stack.function.tp(v), &Context::Variable);
            let new_end = stack.position + v_size;
            let v_scope = stack.function.scope(v);
            let v_first = stack.function.first_def(v);
            let v_last = stack.function.last_use(v);
            let has_child_overlap = (0..stack.function.count()).any(|j| {
                if j == v || stack.function.stack(j) == u16::MAX {
                    return false;
                }
                let j_scope = stack.function.scope(j);
                if j_scope == v_scope {
                    return false;
                }
                let js = stack.function.stack(j);
                let je = js + size(stack.function.tp(j), &Context::Variable);
                stack.position < je
                    && new_end > js
                    && v_first <= stack.function.last_use(j)
                    && v_last >= stack.function.first_def(j)
            });
            if !has_child_overlap {
                stack.function.set_stack_pos(v, stack.position);
            }
        }
    }

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
            // Step 8 fix: place_large_and_recurse processes the inner Block at v's slot, so
            // outer_var and inner_var share the block-return slot — pos == stack.position always.
            // Guard: if this fires, a new Set(v, Block) pattern bypassed the Step-8 fix.
            #[cfg(debug_assertions)]
            debug_assert!(
                pos <= stack.position,
                "[generate_set] Step-8 regression: pos({pos}) > stack.position({}) for '{}' \
                 in '{}' — a Set(v, Block) pattern was not handled by place_large_and_recurse",
                stack.position,
                stack.function.name(v),
                stack.data.def(stack.def_nr).name,
            );
            Self::adjust_first_assignment_slot(stack, v, pos);
            let pos = stack.function.stack(v);
            if pos == stack.position {
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
                    stack.add_op("OpConvRefFromNull", self);
                    stack.add_op("OpDatabase", self);
                    self.code_add(size_of::<crate::keys::DbRef>() as u16);
                    let tp_nr = stack.data.def(d_nr).known_type;
                    self.code_add(tp_nr);
                    self.generate(value, stack, false);
                } else if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
                    && let Value::Var(src) = value
                    && let Type::Reference(src_d_nr, _) = stack.function.tp(*src)
                    && d_nr == *src_d_nr
                {
                    // First assignment `d = c` where both are owned References to the same struct:
                    // give d its own independent record by allocating storage and copying c's data.
                    let src = *src;
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
                } else if matches!(stack.function.tp(v), Type::Vector(_, _))
                    && *value == Value::Null
                {
                    self.gen_set_first_vector_null(stack, v);
                } else {
                    self.generate(value, stack, false);
                }
            } else {
                // Slot is below current TOS — primitive reusing a dead variable's slot.
                // Use set_var() so the value is generated at TOS then stored at pos via OpPutX.
                debug_assert!(pos < stack.position);
                // Text variables MUST be initialised with OpText (direct placement) before any
                // OpAppendText call.  If a Text variable lands here (pos < TOS) it means
                // assign_slots under-estimated the physical TOS at first_def: the pre-assigned
                // slot is below an already-live evaluation-stack value, so OpText was never
                // emitted and OpAppendText will dereference garbage → SIGSEGV at runtime.
                // When this assert fires, fix assign_slots to raise tos_estimate so the Text
                // variable is placed at the correct physical TOS (where pos == stack.position).
                debug_assert!(
                    !matches!(stack.function.tp(v), Type::Text(_)),
                    "[generate_set] Text variable '{}' (var={v}) in '{}': \
                     pre-assigned slot {pos} < TOS {} — OpText not emitted, \
                     OpAppendText would corrupt the stack. \
                     Fix: raise tos_estimate in assign_slots so Text lands at TOS.",
                    stack.function.name(v),
                    stack.data.def(stack.def_nr).name,
                    stack.position,
                );
                self.set_var(stack, v, value);
            }
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

    /// destination-passing for text-producing natives inside `OpAppendText`.
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
        // try destination-passing optimisation for text-producing natives.
        if self.try_text_dest_pass(stack, op, parameters) {
            return Type::Void;
        }
        for (a_nr, a) in stack.data.def(op).attributes.iter().enumerate() {
            if a.mutable {
                #[cfg(debug_assertions)]
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
        if stack.data.def(op).name == "n_parallel_for" {
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
            self.code_add(stack.data.def(op).op_code as u8);
            self.code_add(value_size);
            // Stack: -12 (DbRef consumed) + value_size (yielded value pushed).
            stack.position -= super::size_ref() as u16;
            stack.position += value_size;
            // Return type is the yield type — inferred from value_size for now.
            return match value_size {
                1 => Type::Boolean,
                8 => Type::Long,
                _ => I32.clone(),
            };
        }
        if name == "OpCoroutineExhausted" && !parameters.is_empty() {
            // parameters[0] is the gen expression — generate it (pushes DbRef, +12).
            self.generate(&parameters[0], stack, false);
            self.remember_stack(stack.position);
            self.code_add(stack.data.def(op).op_code as u8);
            // Stack: -12 (DbRef consumed) + 1 (bool pushed).
            stack.position -= super::size_ref() as u16;
            stack.position += 1;
            return Type::Boolean;
        }
        if stack.data.def(op).is_operator() {
            let before_stack = stack.position;
            self.remember_stack(stack.position);
            let code = self.code_pos;
            self.code_add(stack.data.def(op).op_code as u8);
            stack.operator(op);
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
        } else if self.library_names.contains_key(&name) {
            stack.add_op("OpStaticCall", self);
            self.code_add(self.library_names[&name]);
            for a in &stack.data.def(op).attributes {
                stack.position -= size(&a.typedef, &Context::Argument);
            }
            // also subtract the extra args pushed beyond declared params.
            if stack.data.def(op).name == "n_parallel_for" {
                let n_declared = stack.data.def(op).attributes.len();
                for extra in parameters.iter().skip(n_declared) {
                    // Extra args are always integer (4 bytes) in the current implementation.
                    let _ = extra;
                    stack.position -= 4;
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
            self.code_add(op); // d_nr: u32
            let args_size: u16 = stack
                .data
                .def(op)
                .attributes
                .iter()
                .map(|a| size(&a.typedef, &Context::Argument))
                .sum();
            self.code_add(args_size);
            self.code_add(stack.data.def(op).code_position as i32);
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
            Type::Reference(t, _) => {
                if t < u32::from(u16::MAX) {
                    self.types.insert(code, stack.data.def(t).known_type);
                }
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
        let Type::Function(param_types, ret_type) = stack.function.tp(v_nr).clone() else {
            panic!("generate_call_ref: variable is not Type::Function");
        };
        let ret_type = *ret_type;
        for arg in args {
            self.generate(arg, stack, false);
        }
        // fn-ref variable is below the pushed arguments; compute distance from current top
        let fn_var_dist = stack.position - stack.function.stack(v_nr);
        let total_arg_size: u16 = param_types
            .iter()
            .map(|t| size(t, &Context::Argument))
            .sum();
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
            Type::Integer(_, _) | Type::Function(_, _) => stack.add_op("OpVarInt", self),
            Type::Character => stack.add_op("OpVarCharacter", self),
            Type::RefVar(_) => stack.add_op("OpVarRef", self),
            Type::Enum(_, false, _) => stack.add_op("OpVarEnum", self),
            Type::Boolean => stack.add_op("OpVarBool", self),
            Type::Long => stack.add_op("OpVarLong", self),
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
                        Type::Integer(_, _) | Type::Function(_, _) => {
                            stack.add_op("OpVarInt", self);
                        }
                        Type::Boolean => stack.add_op("OpVarBool", self),
                        Type::Long => stack.add_op("OpVarLong", self),
                        Type::Float => stack.add_op("OpVarFloat", self),
                        Type::Single => stack.add_op("OpVarSingle", self),
                        Type::Character => stack.add_op("OpVarCharacter", self),
                        Type::Enum(_, false, _) => stack.add_op("OpVarEnum", self),
                        Type::Text(_) => stack.add_op("OpVarText", self),
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
                Type::Integer(_, _) => stack.add_op("OpGetInt", self),
                Type::Character => stack.add_op("OpGetCharacter", self),
                Type::Long => stack.add_op("OpGetLong", self),
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
                has_return = false;
                return_expr = 0;
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
        if let Type::Reference(c, _) = tp {
            stack.data.def(*c).known_type
        } else {
            self.database.name(&tp.name(stack.data))
        }
    }

    pub(super) fn add_const(&mut self, tp: &Type, p: &Value, stack: &Stack, before_stack: u16) {
        match tp {
            Type::Integer(0, 255) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as u8);
                }
            }
            Type::Integer(-128, 127) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as i8);
                }
            }
            Type::Integer(0, 65535) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as u16);
                } else if let Value::Var(v) = p {
                    let r = stack.function.stack(*v);
                    self.code_add(before_stack - r);
                }
            }
            Type::Integer(-32768, 32767) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as i16);
                }
            }
            Type::Integer(_, _) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr);
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
            Type::Long => {
                if let Value::Long(val) = p {
                    self.code_add(*val);
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
                Type::Integer(_, _) => stack.add_op("OpSetInt", self),
                Type::Character => stack.add_op("OpSetCharacter", self),
                Type::Long => stack.add_op("OpSetLong", self),
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
            Type::Integer(_, _) | Type::Function(_, _) => stack.add_op("OpPutInt", self),
            Type::Character => stack.add_op("OpPutCharacter", self),
            Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
            Type::Boolean => stack.add_op("OpPutBool", self),
            Type::Long => stack.add_op("OpPutLong", self),
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
                        Type::Integer(_, _) | Type::Function(_, _) => {
                            stack.add_op("OpPutInt", self);
                        }
                        Type::Boolean => stack.add_op("OpPutBool", self),
                        Type::Long => stack.add_op("OpPutLong", self),
                        Type::Float => stack.add_op("OpPutFloat", self),
                        Type::Single => stack.add_op("OpPutSingle", self),
                        Type::Character => stack.add_op("OpPutCharacter", self),
                        Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
                        Type::Text(_) => stack.add_op("OpAppendText", self),
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

    /// emit a destination-passing call for a text-returning native function.
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

/// Recursively checks whether `value` contains a direct `Var(v)` reference.
/// Used in debug builds to detect first-assignment self-reference bugs: if
/// `Set(v, expr)` and `expr` contains `Var(v)`, the variable is used before
/// its storage has been allocated — almost always a parser-level bug.
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
        Value::Yield(inner) => {
            eprint!("yield ");
            print_ir(inner, data, vars, depth);
        }
    }
}
