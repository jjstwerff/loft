#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(unused_parens)]

use crate::keys::{DbRef, Str};
use crate::ops;
use crate::state::State;
use crate::vector;

pub const OPERATORS: &[fn(&mut State); 253] = &[
    goto,
    goto_word,
    goto_false,
    goto_false_word,
    call,
    op_return,
    free_stack,
    reserve_frame,
    const_true,
    const_false,
    cast_text_from_bool,
    var_bool,
    put_bool,
    not,
    const_int,
    const_short,
    const_tiny,
    var_int,
    var_character,
    put_int,
    put_character,
    conv_int_from_null,
    conv_character_from_null,
    const_long_text,
    cast_int_from_text,
    cast_long_from_text,
    cast_single_from_text,
    cast_float_from_text,
    abs_int,
    min_single_int,
    conv_long_from_int,
    conv_float_from_int,
    conv_single_from_int,
    conv_bool_from_int,
    add_int,
    min_int,
    mul_int,
    div_int,
    rem_int,
    land_int,
    lor_int,
    eor_int,
    s_left_int,
    s_right_int,
    eq_int,
    ne_int,
    lt_int,
    le_int,
    const_long,
    var_long,
    put_long,
    conv_long_from_null,
    abs_long,
    min_single_long,
    cast_int_from_long,
    conv_float_from_long,
    conv_bool_from_long,
    add_long,
    min_long,
    mul_long,
    div_long,
    rem_long,
    land_long,
    lor_long,
    eor_long,
    s_left_long,
    s_right_long,
    eq_long,
    ne_long,
    lt_long,
    le_long,
    format_long,
    format_stack_long,
    const_single,
    var_single,
    put_single,
    conv_single_from_null,
    abs_single,
    min_single_single,
    cast_int_from_single,
    cast_long_from_single,
    conv_float_from_single,
    conv_bool_from_single,
    add_single,
    min_single,
    mul_single,
    div_single,
    rem_single,
    math_func_single,
    math_func2_single,
    pow_single,
    eq_single,
    ne_single,
    lt_single,
    le_single,
    format_single,
    format_stack_single,
    const_float,
    var_float,
    put_float,
    conv_float_from_null,
    abs_float,
    math_pi_float,
    math_e_float,
    math_func_float,
    math_func2_float,
    pow_float,
    min_single_float,
    cast_single_from_float,
    cast_int_from_float,
    cast_long_from_float,
    conv_bool_from_float,
    add_float,
    min_float,
    mul_float,
    div_float,
    rem_float,
    eq_float,
    ne_float,
    lt_float,
    le_float,
    format_float,
    format_stack_float,
    var_text,
    arg_text,
    const_text,
    conv_text_from_null,
    length_text,
    size_text,
    length_character,
    conv_bool_from_text,
    text,
    append_text,
    get_text_sub,
    text_character,
    conv_bool_from_character,
    clear_text,
    free_text,
    eq_text,
    ne_text,
    lt_text,
    le_text,
    format_text,
    format_stack_text,
    append_character,
    text_compare,
    cast_character_from_int,
    conv_int_from_character,
    var_enum,
    const_enum,
    put_enum,
    conv_bool_from_enum,
    cast_text_from_enum,
    cast_enum_from_text,
    conv_int_from_enum,
    cast_enum_from_int,
    conv_enum_from_null,
    database,
    format_database,
    format_stack_database,
    conv_bool_from_ref,
    conv_ref_from_null,
    null_ref_sentinel,
    free_ref,
    sizeof_ref,
    var_ref,
    put_ref,
    eq_ref,
    ne_ref,
    get_ref,
    set_ref,
    get_field,
    get_int,
    get_character,
    get_long,
    get_single,
    get_float,
    get_byte,
    get_enum,
    set_enum,
    get_short,
    get_text,
    set_int,
    set_character,
    set_long,
    set_single,
    set_float,
    set_byte,
    set_short,
    set_text,
    var_vector,
    length_vector,
    clear_vector,
    get_vector,
    vector_ref,
    cast_vector_from_text,
    remove_vector,
    insert_vector,
    new_record,
    finish_record,
    append_vector,
    get_record,
    validate,
    hash_add,
    hash_find,
    hash_remove,
    eq_bool,
    ne_bool,
    panic,
    print,
    iterate,
    step,
    remove,
    clear,
    append_copy,
    copy_record,
    static_call,
    create_stack,
    get_stack_text,
    get_stack_ref,
    set_stack_ref,
    append_stack_text,
    append_stack_character,
    clear_stack_text,
    si_load2_add_store,
    si_load_const_add_store,
    si_load_const_cmp_branch,
    si_load2_cmp_branch,
    si_load_const_mul_store,
    si_load2_mul_store,
    nop,
    get_file,
    get_dir,
    get_png_image,
    get_file_text,
    write_file,
    read_file,
    seek_file,
    size_file,
    delete,
    move_file,
    truncate_file,
    call_ref,
    mkdir,
    mkdir_all,
    clear_scratch,
    reverse_vector,
    sort_vector,
    coroutine_create,
    coroutine_next,
    coroutine_return,
    coroutine_yield,
    coroutine_exhausted,
];

fn goto(s: &mut State) {
    let v_step = *s.code::<i8>();
    s.code_pos = (s.code_pos as i32 + i32::from(v_step)) as u32;
}

fn goto_word(s: &mut State) {
    let v_step = *s.code::<i16>();
    s.code_pos = (s.code_pos as i32 + i32::from(v_step)) as u32;
}

fn goto_false(s: &mut State) {
    let v_step = *s.code::<i8>();
    let v_if_false = *s.get_stack::<bool>();
    if !v_if_false {
        s.code_pos = (s.code_pos as i32 + i32::from(v_step)) as u32;
    }
}

fn goto_false_word(s: &mut State) {
    let v_step = *s.code::<i16>();
    let v_if_false = *s.get_stack::<bool>();
    if !v_if_false {
        s.code_pos = (s.code_pos as i32 + i32::from(v_step)) as u32;
    }
}

fn call(s: &mut State) {
    let v_d_nr = *s.code::<i32>();
    let v_args_size = *s.code::<u16>();
    let v_to = *s.code::<i32>();
    s.fn_call(v_d_nr as u32, v_args_size, v_to);
}

fn op_return(s: &mut State) {
    let v_ret = *s.code::<u16>();
    let v_value = *s.code::<u8>();
    let v_discard = *s.code::<u16>();
    s.fn_return(v_ret, v_value, v_discard);
}

fn free_stack(s: &mut State) {
    let v_value = *s.code::<u8>();
    let v_discard = *s.code::<u16>();
    s.free_stack(v_value, v_discard);
}

fn reserve_frame(s: &mut State) {
    let v_size = *s.code::<u16>();
    s.reserve_frame(v_size);
}

fn const_true(s: &mut State) {
    let new_value = true;
    s.put_stack(new_value);
}

fn const_false(s: &mut State) {
    let new_value = false;
    s.put_stack(new_value);
}

fn cast_text_from_bool(s: &mut State) {
    let v_v1 = *s.get_stack::<bool>();
    let new_value = if v_v1 { "true" } else { "false" };
    s.put_stack(new_value);
}

fn var_bool(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let new_value = *s.get_var::<bool>(v_pos);
    s.put_stack(new_value);
}

fn put_bool(s: &mut State) {
    let v_var = *s.code::<u16>();
    let v_value = *s.get_stack::<bool>();
    s.put_var(v_var, v_value);
}

fn not(s: &mut State) {
    let v_v1 = *s.get_stack::<bool>();
    let new_value = !v_v1;
    s.put_stack(new_value);
}

fn const_int(s: &mut State) {
    let v_val = *s.code::<i32>();
    let new_value = v_val;
    s.put_stack(new_value);
}

fn const_short(s: &mut State) {
    let v_val = *s.code::<i16>();
    let new_value = i32::from(v_val);
    s.put_stack(new_value);
}

fn const_tiny(s: &mut State) {
    let v_val = *s.code::<i8>();
    let new_value = i32::from(v_val);
    s.put_stack(new_value);
}

fn var_int(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let new_value = *s.get_var::<i32>(v_pos);
    s.put_stack(new_value);
}

fn var_character(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let new_value = *s.get_var::<char>(v_pos);
    s.put_stack(new_value);
}

fn put_int(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let v_value = *s.get_stack::<i32>();
    s.put_var(v_pos, v_value);
}

fn put_character(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let v_value = *s.get_stack::<char>();
    s.put_var(v_pos, v_value);
}

fn conv_int_from_null(s: &mut State) {
    let new_value = i32::MIN;
    s.put_stack(new_value);
}

fn conv_character_from_null(s: &mut State) {
    let new_value = char::from(0);
    s.put_stack(new_value);
}

fn const_long_text(s: &mut State) {
    let v_start = *s.code::<i32>();
    let v_size = *s.code::<i32>();
    s.string_from_texts(v_start, v_size);
}

fn cast_int_from_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().parse().unwrap_or(i32::MIN);
    s.put_stack(new_value);
}

fn cast_long_from_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().parse().unwrap_or(i64::MIN);
    s.put_stack(new_value);
}

fn cast_single_from_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().parse().unwrap_or(f32::NAN);
    s.put_stack(new_value);
}

fn cast_float_from_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().parse().unwrap_or(f64::NAN);
    s.put_stack(new_value);
}

fn abs_int(s: &mut State) {
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_abs_int(v_v1);
    s.put_stack(new_value);
}

fn min_single_int(s: &mut State) {
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_negate_int(v_v1);
    s.put_stack(new_value);
}

fn conv_long_from_int(s: &mut State) {
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_conv_long_from_int(v_v1);
    s.put_stack(new_value);
}

fn conv_float_from_int(s: &mut State) {
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_conv_float_from_int(v_v1);
    s.put_stack(new_value);
}

fn conv_single_from_int(s: &mut State) {
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_conv_single_from_int(v_v1);
    s.put_stack(new_value);
}

fn conv_bool_from_int(s: &mut State) {
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_conv_bool_from_int(v_v1);
    s.put_stack(new_value);
}

fn add_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_add_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn min_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_min_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn mul_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_mul_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn div_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_div_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn rem_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_rem_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn land_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_logical_and_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn lor_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_logical_or_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn eor_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_exclusive_or_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn s_left_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_shift_left_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn s_right_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = ops::op_shift_right_int(v_v1, v_v2);
    s.put_stack(new_value);
}

fn eq_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = v_v1 == v_v2;
    s.put_stack(new_value);
}

fn ne_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = v_v1 != v_v2;
    s.put_stack(new_value);
}

fn lt_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = v_v1 < v_v2;
    s.put_stack(new_value);
}

fn le_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<i32>();
    let new_value = v_v1 <= v_v2;
    s.put_stack(new_value);
}

fn const_long(s: &mut State) {
    let v_val = *s.code::<i64>();
    let new_value = v_val;
    s.put_stack(new_value);
}

fn var_long(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let new_value = *s.get_var::<i64>(v_pos);
    s.put_stack(new_value);
}

fn put_long(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let v_value = *s.get_stack::<i64>();
    s.put_var(v_pos, v_value);
}

fn conv_long_from_null(s: &mut State) {
    let new_value = i64::MIN;
    s.put_stack(new_value);
}

fn abs_long(s: &mut State) {
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_abs_long(v_v1);
    s.put_stack(new_value);
}

fn min_single_long(s: &mut State) {
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_negate_long(v_v1);
    s.put_stack(new_value);
}

fn cast_int_from_long(s: &mut State) {
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_cast_int_from_long(v_v1);
    s.put_stack(new_value);
}

fn conv_float_from_long(s: &mut State) {
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_conv_float_from_long(v_v1);
    s.put_stack(new_value);
}

fn conv_bool_from_long(s: &mut State) {
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_conv_bool_from_long(v_v1);
    s.put_stack(new_value);
}

fn add_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_add_long(v_v1, v_v2);
    s.put_stack(new_value);
}

fn min_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_min_long(v_v1, v_v2);
    s.put_stack(new_value);
}

fn mul_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_mul_long(v_v1, v_v2);
    s.put_stack(new_value);
}

fn div_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_div_long(v_v1, v_v2);
    s.put_stack(new_value);
}

fn rem_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_rem_long(v_v1, v_v2);
    s.put_stack(new_value);
}

fn land_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_logical_and_long(v_v1, v_v2);
    s.put_stack(new_value);
}

fn lor_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_logical_or_long(v_v1, v_v2);
    s.put_stack(new_value);
}

fn eor_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_exclusive_or_long(v_v1, v_v2);
    s.put_stack(new_value);
}

fn s_left_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_shift_left_long(v_v1, v_v2);
    s.put_stack(new_value);
}

fn s_right_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_shift_right_long(v_v1, v_v2);
    s.put_stack(new_value);
}

fn eq_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = v_v1 == v_v2;
    s.put_stack(new_value);
}

fn ne_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = v_v1 != v_v2;
    s.put_stack(new_value);
}

fn lt_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = v_v1 < v_v2;
    s.put_stack(new_value);
}

fn le_long(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = v_v1 <= v_v2;
    s.put_stack(new_value);
}

fn format_long(s: &mut State) {
    s.format_long();
}

fn format_stack_long(s: &mut State) {
    s.format_stack_long();
}

fn const_single(s: &mut State) {
    let v_val = *s.code::<f32>();
    let new_value = v_val;
    s.put_stack(new_value);
}

fn var_single(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let new_value = *s.get_var::<f32>(v_pos);
    s.put_stack(new_value);
}

fn put_single(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let v_value = *s.get_stack::<f32>();
    s.put_var(v_pos, v_value);
}

fn conv_single_from_null(s: &mut State) {
    let new_value = f32::NAN;
    s.put_stack(new_value);
}

fn abs_single(s: &mut State) {
    let v_v1 = *s.get_stack::<f32>();
    let new_value = v_v1.abs();
    s.put_stack(new_value);
}

fn min_single_single(s: &mut State) {
    let v_v1 = *s.get_stack::<f32>();
    let new_value = -v_v1;
    s.put_stack(new_value);
}

fn cast_int_from_single(s: &mut State) {
    let v_v1 = *s.get_stack::<f32>();
    let new_value = ops::op_cast_int_from_single(v_v1);
    s.put_stack(new_value);
}

fn cast_long_from_single(s: &mut State) {
    let v_v1 = *s.get_stack::<f32>();
    let new_value = ops::op_cast_long_from_single(v_v1);
    s.put_stack(new_value);
}

fn conv_float_from_single(s: &mut State) {
    let v_v1 = *s.get_stack::<f32>();
    let new_value = f64::from(v_v1);
    s.put_stack(new_value);
}

fn conv_bool_from_single(s: &mut State) {
    let v_v1 = *s.get_stack::<f32>();
    let new_value = !v_v1.is_nan();
    s.put_stack(new_value);
}

fn add_single(s: &mut State) {
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = v_v1 + v_v2;
    s.put_stack(new_value);
}

fn min_single(s: &mut State) {
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = v_v1 - v_v2;
    s.put_stack(new_value);
}

fn mul_single(s: &mut State) {
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = v_v1 * v_v2;
    s.put_stack(new_value);
}

fn div_single(s: &mut State) {
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = v_v1 / v_v2;
    s.put_stack(new_value);
}

fn rem_single(s: &mut State) {
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = v_v1 % v_v2;
    s.put_stack(new_value);
}

fn math_func_single(s: &mut State) {
    let v_fn_id = *s.code::<i8>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = match v_fn_id {
        0 => v_v1.cos(),
        1 => v_v1.sin(),
        2 => v_v1.tan(),
        3 => v_v1.acos(),
        4 => v_v1.asin(),
        5 => v_v1.atan(),
        6 => v_v1.ceil(),
        7 => v_v1.floor(),
        8 => v_v1.round(),
        9 => v_v1.sqrt(),
        _ => f32::NAN,
    };
    s.put_stack(new_value);
}

fn math_func2_single(s: &mut State) {
    let v_fn_id = *s.code::<i8>();
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = match v_fn_id {
        0 => v_v1.atan2(v_v2),
        1 => v_v1.log(v_v2),
        _ => f32::NAN,
    };
    s.put_stack(new_value);
}

fn pow_single(s: &mut State) {
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = v_v1.powf(v_v2);
    s.put_stack(new_value);
}

fn eq_single(s: &mut State) {
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = !v_v1.is_nan() && !v_v2.is_nan() && (v_v1 - v_v2).abs() < 0.000_001f32;
    s.put_stack(new_value);
}

fn ne_single(s: &mut State) {
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = v_v1.is_nan() || v_v2.is_nan() || (v_v1 - v_v2).abs() > 0.000_001f32;
    s.put_stack(new_value);
}

fn lt_single(s: &mut State) {
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = !v_v1.is_nan() && !v_v2.is_nan() && v_v1 < v_v2;
    s.put_stack(new_value);
}

fn le_single(s: &mut State) {
    let v_v2 = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<f32>();
    let new_value = !v_v1.is_nan() && !v_v2.is_nan() && v_v1 <= v_v2;
    s.put_stack(new_value);
}

fn format_single(s: &mut State) {
    s.format_single();
}

fn format_stack_single(s: &mut State) {
    s.format_stack_single();
}

fn const_float(s: &mut State) {
    let v_val = *s.code::<f64>();
    let new_value = v_val;
    s.put_stack(new_value);
}

fn var_float(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let new_value = *s.get_var::<f64>(v_pos);
    s.put_stack(new_value);
}

fn put_float(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let v_value = *s.get_stack::<f64>();
    s.put_var(v_pos, v_value);
}

fn conv_float_from_null(s: &mut State) {
    let new_value = f64::NAN;
    s.put_stack(new_value);
}

fn abs_float(s: &mut State) {
    let v_v1 = *s.get_stack::<f64>();
    let new_value = v_v1.abs();
    s.put_stack(new_value);
}

fn math_pi_float(s: &mut State) {
    let new_value = std::f64::consts::PI;
    s.put_stack(new_value);
}

fn math_e_float(s: &mut State) {
    let new_value = std::f64::consts::E;
    s.put_stack(new_value);
}

fn math_func_float(s: &mut State) {
    let v_fn_id = *s.code::<i8>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = match v_fn_id {
        0 => v_v1.cos(),
        1 => v_v1.sin(),
        2 => v_v1.tan(),
        3 => v_v1.acos(),
        4 => v_v1.asin(),
        5 => v_v1.atan(),
        6 => v_v1.ceil(),
        7 => v_v1.floor(),
        8 => v_v1.round(),
        9 => v_v1.sqrt(),
        _ => f64::NAN,
    };
    s.put_stack(new_value);
}

fn math_func2_float(s: &mut State) {
    let v_fn_id = *s.code::<i8>();
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = match v_fn_id {
        0 => v_v1.atan2(v_v2),
        1 => v_v1.log(v_v2),
        _ => f64::NAN,
    };
    s.put_stack(new_value);
}

fn pow_float(s: &mut State) {
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = v_v1.powf(v_v2);
    s.put_stack(new_value);
}

fn min_single_float(s: &mut State) {
    let v_v1 = *s.get_stack::<f64>();
    let new_value = -v_v1;
    s.put_stack(new_value);
}

fn cast_single_from_float(s: &mut State) {
    let v_v1 = *s.get_stack::<f64>();
    let new_value = v_v1 as f32;
    s.put_stack(new_value);
}

fn cast_int_from_float(s: &mut State) {
    let v_v1 = *s.get_stack::<f64>();
    let new_value = ops::op_cast_int_from_float(v_v1);
    s.put_stack(new_value);
}

fn cast_long_from_float(s: &mut State) {
    let v_v1 = *s.get_stack::<f64>();
    let new_value = ops::op_cast_long_from_float(v_v1);
    s.put_stack(new_value);
}

fn conv_bool_from_float(s: &mut State) {
    let v_v1 = *s.get_stack::<f64>();
    let new_value = !v_v1.is_nan();
    s.put_stack(new_value);
}

fn add_float(s: &mut State) {
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = v_v1 + v_v2;
    s.put_stack(new_value);
}

fn min_float(s: &mut State) {
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = v_v1 - v_v2;
    s.put_stack(new_value);
}

fn mul_float(s: &mut State) {
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = v_v1 * v_v2;
    s.put_stack(new_value);
}

fn div_float(s: &mut State) {
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = v_v1 / v_v2;
    s.put_stack(new_value);
}

fn rem_float(s: &mut State) {
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = v_v1 % v_v2;
    s.put_stack(new_value);
}

fn eq_float(s: &mut State) {
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = !v_v1.is_nan() && !v_v2.is_nan() && (v_v1 - v_v2).abs() < 0.000_000_001f64;
    s.put_stack(new_value);
}

fn ne_float(s: &mut State) {
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = v_v1.is_nan() || v_v2.is_nan() || (v_v1 - v_v2).abs() > 0.000_000_001f64;
    s.put_stack(new_value);
}

fn lt_float(s: &mut State) {
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = !v_v1.is_nan() && !v_v2.is_nan() && v_v1 < v_v2;
    s.put_stack(new_value);
}

fn le_float(s: &mut State) {
    let v_v2 = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<f64>();
    let new_value = !v_v1.is_nan() && !v_v2.is_nan() && v_v1 <= v_v2;
    s.put_stack(new_value);
}

fn format_float(s: &mut State) {
    s.format_float();
}

fn format_stack_float(s: &mut State) {
    s.format_stack_float();
}

fn var_text(s: &mut State) {
    s.var_text();
}

fn arg_text(s: &mut State) {
    s.arg_text();
}

fn const_text(s: &mut State) {
    s.string_from_code();
}

fn conv_text_from_null(s: &mut State) {
    s.conv_text_from_null();
}

fn length_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().len() as i32;
    s.put_stack(new_value);
}

fn size_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str().chars().count() as i32;
    s.put_stack(new_value);
}

fn length_character(s: &mut State) {
    s.length_character();
}

fn conv_bool_from_text(s: &mut State) {
    let v_v1 = s.string();
    let new_value = v_v1.str() != crate::state::STRING_NULL;
    s.put_stack(new_value);
}

fn text(s: &mut State) {
    s.text();
}

fn append_text(s: &mut State) {
    s.append_text();
}

fn get_text_sub(s: &mut State) {
    s.get_text_sub();
}

fn text_character(s: &mut State) {
    let v_v2 = *s.get_stack::<i32>();
    let v_v1 = s.string();
    let new_value = ops::text_character(v_v1.str(), v_v2);
    s.put_stack(new_value);
}

fn conv_bool_from_character(s: &mut State) {
    let v_v1 = *s.get_stack::<char>();
    let new_value = ops::op_conv_bool_from_character(v_v1);
    s.put_stack(new_value);
}

fn clear_text(s: &mut State) {
    s.clear_text();
}

fn free_text(s: &mut State) {
    s.free_text();
}

fn eq_text(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = v_v1.str() == v_v2.str();
    s.put_stack(new_value);
}

fn ne_text(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = v_v1.str() != v_v2.str();
    s.put_stack(new_value);
}

fn lt_text(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = v_v1.str() < v_v2.str();
    s.put_stack(new_value);
}

fn le_text(s: &mut State) {
    let v_v2 = s.string();
    let v_v1 = s.string();
    let new_value = v_v1.str() <= v_v2.str();
    s.put_stack(new_value);
}

fn format_text(s: &mut State) {
    s.format_text();
}

fn format_stack_text(s: &mut State) {
    s.format_stack_text();
}

fn append_character(s: &mut State) {
    s.append_character();
}

fn text_compare(s: &mut State) {
    s.text_compare();
}

fn cast_character_from_int(s: &mut State) {
    let v_v1 = *s.get_stack::<i32>();
    let new_value = char::from_u32(v_v1 as u32).unwrap_or(char::from(0));
    s.put_stack(new_value);
}

fn conv_int_from_character(s: &mut State) {
    let v_v1 = *s.get_stack::<char>();
    let new_value = if v_v1 == char::from(0) {
        i32::MIN
    } else {
        v_v1 as i32
    };
    s.put_stack(new_value);
}

fn var_enum(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let new_value = *s.get_var::<u8>(v_pos);
    s.put_stack(new_value);
}

fn const_enum(s: &mut State) {
    let v_val = *s.code::<u8>();
    let new_value = v_val;
    s.put_stack(new_value);
}

fn put_enum(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let v_value = *s.get_stack::<u8>();
    s.put_var(v_pos, v_value);
}

fn conv_bool_from_enum(s: &mut State) {
    let v_v1 = *s.get_stack::<u8>();
    let new_value = v_v1 != 255;
    s.put_stack(new_value);
}

fn cast_text_from_enum(s: &mut State) {
    let v_enum_tp = *s.code::<u16>();
    let v_v1 = *s.get_stack::<u8>();
    let new_value = Str::new(s.database.enum_val(v_enum_tp, v_v1));
    s.put_stack(new_value);
}

fn cast_enum_from_text(s: &mut State) {
    let v_enum_tp = *s.code::<u16>();
    let v_v1 = s.string();
    let new_value = s.database.to_enum(v_enum_tp, v_v1.str());
    s.put_stack(new_value);
}

fn conv_int_from_enum(s: &mut State) {
    let v_v1 = *s.get_stack::<u8>();
    let new_value = if v_v1 == 255 {
        i32::MIN
    } else {
        i32::from(v_v1)
    };
    s.put_stack(new_value);
}

fn cast_enum_from_int(s: &mut State) {
    let v_v1 = *s.get_stack::<i32>();
    let new_value = if v_v1 == i32::MIN { 255 } else { v_v1 as u8 };
    s.put_stack(new_value);
}

fn conv_enum_from_null(s: &mut State) {
    let new_value = 255u8;
    s.put_stack(new_value);
}

fn database(s: &mut State) {
    s.database();
}

fn format_database(s: &mut State) {
    s.format_database();
}

fn format_stack_database(s: &mut State) {
    s.format_stack_database();
}

fn conv_bool_from_ref(s: &mut State) {
    let v_val = *s.get_stack::<DbRef>();
    let new_value = v_val.rec != 0;
    s.put_stack(new_value);
}

fn conv_ref_from_null(s: &mut State) {
    let new_value = s.database.null();
    s.put_stack(new_value);
}

fn null_ref_sentinel(s: &mut State) {
    let new_value = DbRef {
        store_nr: u16::MAX,
        rec: 0,
        pos: 0,
    };
    s.put_stack(new_value);
}

fn free_ref(s: &mut State) {
    s.free_ref();
}

fn sizeof_ref(s: &mut State) {
    s.sizeof_ref();
}

fn var_ref(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let new_value = {
        let r = *s.get_var::<DbRef>(v_pos);
        s.database.valid(&r);
        r
    };
    s.put_stack(new_value);
}

fn put_ref(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let v_value = *s.get_stack::<DbRef>();
    s.put_var(v_pos, v_value);
}

fn eq_ref(s: &mut State) {
    let v_v2 = *s.get_stack::<DbRef>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = if v_v1.rec == 0 || v_v2.rec == 0 {
        v_v1.rec == 0 && v_v2.rec == 0
    } else {
        v_v1 == v_v2
    };
    s.put_stack(new_value);
}

fn ne_ref(s: &mut State) {
    let v_v2 = *s.get_stack::<DbRef>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = if v_v1.rec == 0 || v_v2.rec == 0 {
        v_v1.rec != 0 || v_v2.rec != 0
    } else {
        v_v1 != v_v2
    };
    s.put_stack(new_value);
}

fn get_ref(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = s.database.get_ref(&v_v1, u32::from(v_fld));
    s.put_stack(new_value);
}

fn set_ref(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_val = *s.get_stack::<DbRef>();
    let v_v1 = *s.get_stack::<DbRef>();
    {
        let db = v_v1;
        s.database
            .store_mut(&db)
            .set_int(db.rec, db.pos + u32::from(v_fld), v_val.rec as i32);
    }
}

fn get_field(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = DbRef {
        store_nr: v_v1.store_nr,
        rec: v_v1.rec,
        pos: v_v1.pos + u32::from(v_fld),
    };
    s.put_stack(new_value);
}

fn get_int(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            i32::MIN
        } else {
            s.database
                .store(&db)
                .get_int(db.rec, db.pos + u32::from(v_fld))
        }
    };
    s.put_stack(new_value);
}

fn get_character(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        if db.rec == 0 {
            char::from(0)
        } else {
            char::from_u32(
                s.database
                    .store(&db)
                    .get_int(db.rec, db.pos + u32::from(v_fld)) as u32,
            )
            .unwrap_or(char::from(0))
        }
    };
    s.put_stack(new_value);
}

fn get_long(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        s.database
            .store(&db)
            .get_long(db.rec, db.pos + u32::from(v_fld))
    };
    s.put_stack(new_value);
}

fn get_single(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        s.database
            .store(&db)
            .get_single(db.rec, db.pos + u32::from(v_fld))
    };
    s.put_stack(new_value);
}

fn get_float(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        s.database
            .store(&db)
            .get_float(db.rec, db.pos + u32::from(v_fld))
    };
    s.put_stack(new_value);
}

fn get_byte(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_min = *s.code::<i16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        s.database
            .store(&db)
            .get_byte(db.rec, db.pos + u32::from(v_fld), i32::from(v_min))
    };
    s.put_stack(new_value);
}

fn get_enum(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        let r = s
            .database
            .store(&db)
            .get_byte(db.rec, db.pos + u32::from(v_fld), 0);
        if r < 0 { 255u8 } else { r as u8 }
    };
    s.put_stack(new_value);
}

fn set_enum(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_val = *s.get_stack::<u8>();
    let v_v1 = *s.get_stack::<DbRef>();
    {
        let db = v_v1;
        s.database
            .store_mut(&db)
            .set_byte(db.rec, db.pos + u32::from(v_fld), 0, i32::from(v_val));
    }
}

fn get_short(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_min = *s.code::<i16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        s.database
            .store(&db)
            .get_short(db.rec, db.pos + u32::from(v_fld), i32::from(v_min))
    };
    s.put_stack(new_value);
}

fn get_text(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_v1 = *s.get_stack::<DbRef>();
    let new_value = {
        let db = v_v1;
        let store = s.database.store(&db);
        Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from(v_fld)) as u32))
    };
    s.put_stack(new_value);
}

fn set_int(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_val = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<DbRef>();
    {
        let db = v_v1;
        s.database
            .store_mut(&db)
            .set_int(db.rec, db.pos + u32::from(v_fld), v_val);
    }
}

fn set_character(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_val = *s.get_stack::<char>();
    let v_v1 = *s.get_stack::<DbRef>();
    {
        let db = v_v1;
        s.database
            .store_mut(&db)
            .set_int(db.rec, db.pos + u32::from(v_fld), v_val as i32);
    }
}

fn set_long(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_val = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<DbRef>();
    {
        let db = v_v1;
        s.database
            .store_mut(&db)
            .set_long(db.rec, db.pos + u32::from(v_fld), v_val);
    }
}

fn set_single(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_val = *s.get_stack::<f32>();
    let v_v1 = *s.get_stack::<DbRef>();
    {
        let db = v_v1;
        s.database
            .store_mut(&db)
            .set_single(db.rec, db.pos + u32::from(v_fld), v_val);
    }
}

fn set_float(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_val = *s.get_stack::<f64>();
    let v_v1 = *s.get_stack::<DbRef>();
    {
        let db = v_v1;
        s.database
            .store_mut(&db)
            .set_float(db.rec, db.pos + u32::from(v_fld), v_val);
    }
}

fn set_byte(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_min = *s.code::<i16>();
    let v_val = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<DbRef>();
    {
        let db = v_v1;
        s.database.store_mut(&db).set_byte(
            db.rec,
            db.pos + u32::from(v_fld),
            i32::from(v_min),
            v_val,
        );
    }
}

fn set_short(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_min = *s.code::<i16>();
    let v_val = *s.get_stack::<i32>();
    let v_v1 = *s.get_stack::<DbRef>();
    {
        let db = v_v1;
        s.database.store_mut(&db).set_short(
            db.rec,
            db.pos + u32::from(v_fld),
            i32::from(v_min),
            v_val,
        );
    }
}

fn set_text(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_val = s.string();
    let v_v1 = *s.get_stack::<DbRef>();
    {
        let db = v_v1;
        let s_val = v_val.str().to_string();
        let store = s.database.store_mut(&db);
        let s_pos = store.set_str(&s_val);
        store.set_int(db.rec, db.pos + u32::from(v_fld), s_pos as i32);
    }
}

fn var_vector(s: &mut State) {
    let v_pos = *s.code::<u16>();
    let new_value = *s.get_var::<DbRef>(v_pos);
    s.put_stack(new_value);
}

fn length_vector(s: &mut State) {
    let v_r = *s.get_stack::<DbRef>();
    let new_value = vector::length_vector(&v_r, &s.database.allocations) as i32;
    s.put_stack(new_value);
}

fn clear_vector(s: &mut State) {
    let v_r = *s.get_stack::<DbRef>();
    vector::clear_vector(&v_r, &mut s.database.allocations);
}

fn get_vector(s: &mut State) {
    let v_size = *s.code::<u16>();
    let v_index = *s.get_stack::<i32>();
    let v_r = *s.get_stack::<DbRef>();
    let new_value = vector::get_vector(&v_r, u32::from(v_size), v_index, &s.database.allocations);
    s.put_stack(new_value);
}

fn vector_ref(s: &mut State) {
    let v_index = *s.get_stack::<i32>();
    let v_r = *s.get_stack::<DbRef>();
    let new_value = s.database.get_ref(
        &vector::get_vector(&v_r, 4, v_index, &s.database.allocations),
        0,
    );
    s.put_stack(new_value);
}

fn cast_vector_from_text(s: &mut State) {
    let v_db_tp = *s.code::<u16>();
    let v_val = s.string();
    let new_value = s.db_from_text(v_val.str(), v_db_tp);
    s.put_stack(new_value);
}

fn remove_vector(s: &mut State) {
    let v_size = *s.code::<u16>();
    let v_index = *s.get_stack::<i32>();
    let v_r = *s.get_stack::<DbRef>();
    let new_value = vector::remove_vector(
        &v_r,
        u32::from(v_size),
        v_index,
        &mut s.database.allocations,
    );
    s.put_stack(new_value);
}

fn insert_vector(s: &mut State) {
    s.insert_vector();
}

fn new_record(s: &mut State) {
    s.new_record();
}

fn finish_record(s: &mut State) {
    s.finish_record();
}

fn append_vector(s: &mut State) {
    let v_tp = *s.code::<u16>();
    let v_other = *s.get_stack::<DbRef>();
    let v_r = *s.get_stack::<DbRef>();
    s.database.vector_add(&v_r, &v_other, v_tp);
}

fn get_record(s: &mut State) {
    s.get_record();
}

fn validate(s: &mut State) {
    s.validate();
}

fn hash_add(s: &mut State) {
    s.hash_add();
}

fn hash_find(s: &mut State) {
    s.hash_find();
}

fn hash_remove(s: &mut State) {
    s.hash_remove();
}

fn eq_bool(s: &mut State) {
    let v_v2 = *s.get_stack::<bool>();
    let v_v1 = *s.get_stack::<bool>();
    let new_value = v_v1 == v_v2;
    s.put_stack(new_value);
}

fn ne_bool(s: &mut State) {
    let v_v2 = *s.get_stack::<bool>();
    let v_v1 = *s.get_stack::<bool>();
    let new_value = v_v1 != v_v2;
    s.put_stack(new_value);
}

fn panic(s: &mut State) {
    let v_message = s.string();
    panic!("{}", v_message.str());
}

fn print(s: &mut State) {
    let v_v1 = s.string();
    print!("{}", v_v1.str());
}

fn iterate(s: &mut State) {
    s.iterate();
}

fn step(s: &mut State) {
    s.step();
}

fn remove(s: &mut State) {
    s.remove();
}

fn clear(s: &mut State) {
    s.clear();
}

fn append_copy(s: &mut State) {
    s.append_copy();
}

fn copy_record(s: &mut State) {
    s.copy_record();
}

fn static_call(s: &mut State) {
    s.static_call();
}

fn create_stack(s: &mut State) {
    s.create_stack();
}

fn get_stack_text(s: &mut State) {
    s.get_stack_text();
}

fn get_stack_ref(s: &mut State) {
    s.get_stack_ref();
}

fn set_stack_ref(s: &mut State) {
    s.set_stack_ref();
}

fn append_stack_text(s: &mut State) {
    s.append_stack_text();
}

fn append_stack_character(s: &mut State) {
    s.append_stack_character();
}

fn clear_stack_text(s: &mut State) {
    s.clear_stack_text();
}

fn si_load2_add_store(s: &mut State) {
    s.si_load2_add_store();
}

fn si_load_const_add_store(s: &mut State) {
    s.si_load_const_add_store();
}

fn si_load_const_cmp_branch(s: &mut State) {
    s.si_load_const_cmp_branch();
}

fn si_load2_cmp_branch(s: &mut State) {
    s.si_load2_cmp_branch();
}

fn si_load_const_mul_store(s: &mut State) {
    s.si_load_const_mul_store();
}

fn si_load2_mul_store(s: &mut State) {
    s.si_load2_mul_store();
}

fn nop(s: &mut State) {
    s.nop();
}

fn get_file(s: &mut State) {
    let v_file = *s.get_stack::<DbRef>();
    let new_value = s.database.get_file(&v_file);
    s.put_stack(new_value);
}

fn get_dir(s: &mut State) {
    let v_result = *s.get_stack::<DbRef>();
    let v_path = s.string();
    let new_value = s.database.get_dir(v_path.str(), &v_result);
    s.put_stack(new_value);
}

fn get_png_image(s: &mut State) {
    let v_image = *s.get_stack::<DbRef>();
    let v_path = s.string();
    let new_value = s.database.get_png(v_path.str(), &v_image);
    s.put_stack(new_value);
}

fn get_file_text(s: &mut State) {
    s.get_file_text();
}

fn write_file(s: &mut State) {
    s.write_file();
}

fn read_file(s: &mut State) {
    s.read_file();
}

fn seek_file(s: &mut State) {
    s.seek_file();
}

fn size_file(s: &mut State) {
    s.size_file();
}

fn delete(s: &mut State) {
    let v_path = s.string();
    let new_value = std::fs::remove_file(v_path.str()).is_ok();
    s.put_stack(new_value);
}

fn move_file(s: &mut State) {
    let v_to = s.string();
    let v_from = s.string();
    let new_value = std::fs::rename(v_from.str(), v_to.str()).is_ok();
    s.put_stack(new_value);
}

fn truncate_file(s: &mut State) {
    s.truncate_file();
}

fn call_ref(s: &mut State) {
    let v_fn_var = *s.code::<u16>();
    let v_arg_size = *s.code::<u16>();
    s.fn_call_ref(v_fn_var, v_arg_size);
}

fn mkdir(s: &mut State) {
    let v_path = s.string();
    let new_value = std::fs::create_dir(v_path.str()).is_ok();
    s.put_stack(new_value);
}

fn mkdir_all(s: &mut State) {
    let v_path = s.string();
    let new_value = std::fs::create_dir_all(v_path.str()).is_ok();
    s.put_stack(new_value);
}

fn clear_scratch(s: &mut State) {
    s.database.scratch.clear();
}

fn reverse_vector(s: &mut State) {
    let v_size = *s.code::<u16>();
    let v_r = *s.get_stack::<DbRef>();
    vector::reverse_vector(&v_r, u32::from(v_size), &mut s.database.allocations);
}

fn sort_vector(s: &mut State) {
    let v_db_tp = *s.code::<u16>();
    let v_r = *s.get_stack::<DbRef>();
    {
        let t = v_db_tp;
        let elem_size = s.database.size(t);
        let is_float = t == 2 || t == 3;
        vector::sort_vector(&v_r, elem_size, is_float, &mut s.database.allocations);
    }
}

fn coroutine_create(s: &mut State) {
    let v_d_nr = *s.code::<i32>();
    let v_args_size = *s.code::<u16>();
    let v_to = *s.code::<i32>();
    s.coroutine_create(v_d_nr as u32, u32::from(v_args_size), v_to as u32);
}

fn coroutine_next(s: &mut State) {
    let v_value_size = *s.code::<u16>();
    s.coroutine_next(u32::from(v_value_size));
}

fn coroutine_return(s: &mut State) {
    let v_value_size = *s.code::<u16>();
    s.coroutine_return(u32::from(v_value_size));
}

fn coroutine_yield(s: &mut State) {
    let v_value_size = *s.code::<u16>();
    s.coroutine_yield(u32::from(v_value_size));
}

fn coroutine_exhausted(s: &mut State) {
    let v_gen = *s.get_stack::<DbRef>();
    let new_value = s.coroutine_exhausted(&v_gen);
    s.put_stack(new_value);
}
