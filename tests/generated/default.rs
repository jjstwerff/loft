#![allow(unused_imports)]
#![allow(unused_parens)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unused_mut)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(redundant_semicolons)]
#![allow(unused_assignments)]
#![allow(clippy::double_parens)]
#![allow(clippy::unused_unit)]

extern crate loft;
use loft::database::Stores;
use loft::keys::{DbRef, Str, Key, Content};
use loft::ops;
use loft::vector;

fn init(db: &mut Stores) {
    db.vector(5);
    let e = db.enumerate("Format");
    db.value(e, "TextFile", u16::MAX);
    db.value(e, "LittleEndian", u16::MAX);
    db.value(e, "BigEndian", u16::MAX);
    db.value(e, "Directory", u16::MAX);
    db.value(e, "NotExists", u16::MAX);
    let s = db.structure("EnvVariable", 0); // 9
    db.field(s, "name", 5);
    db.field(s, "value", 5);
    let s = db.structure("Pixel", 0); // 10
    let byte_r = db.byte(0, false);
    db.field(s, "r", byte_r);
    let byte_g = db.byte(0, false);
    db.field(s, "g", byte_g);
    let byte_b = db.byte(0, false);
    db.field(s, "b", byte_b);
    let s = db.structure("Image", 0); // 12
    db.field(s, "name", 5);
    db.field(s, "width", 0);
    db.field(s, "height", 0);
    let vec_data = db.vector(10);
    db.field(s, "data", vec_data);
    db.vector(10);
    let s = db.structure("File", 0); // 14
    db.field(s, "path", 5);
    db.field(s, "size", 1);
    db.field(s, "format", 8);
    db.field(s, "ref", 0);
    db.field(s, "current", 1);
    db.field(s, "next", 1);
    let s = db.structure("main_vector<text>", 0); // 15
    let vec_vector = db.vector(5);
    db.field(s, "vector", vec_vector);
    let s = db.structure("main_vector<File>", 0); // 16
    let vec_vector = db.vector(14);
    db.field(s, "vector", vec_vector);
    db.vector(14);
    db.finish();
}

fn t_7integer_abs(stores: &mut Stores, mut var_both: i32) -> i32 { //block_1: integer
  external::op_abs_int((var_both))
  } /*block_1: integer*/

fn t_4long_abs(stores: &mut Stores, mut var_both: i64) -> i64 { //block_1: long
  external::op_abs_long((var_both))
  } /*block_1: long*/

fn OpFormatLong(stores: &mut Stores, mut var_pos: u16, mut var_val: i64, mut var_radix: u8, mut var_width: i32, mut var_token: u8, mut var_plus: bool, mut var_note: bool) {

}


fn OpFormatStackLong(stores: &mut Stores, mut var_pos: u16, mut var_val: i64, mut var_radix: u8, mut var_width: i32, mut var_token: u8, mut var_plus: bool, mut var_note: bool) {

}


fn t_6single_abs(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  (var_both).abs()
  } /*block_1: single*/

fn OpMathFuncSingle(stores: &mut Stores, mut var_fn_id: i8, mut var_v1: f32) -> f32 {

}


fn t_6single_cos(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  OpMathFuncSingle(stores, 0_i32, var_both)
  } /*block_1: single*/

fn t_6single_sin(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  OpMathFuncSingle(stores, 1_i32, var_both)
  } /*block_1: single*/

fn t_6single_tan(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  OpMathFuncSingle(stores, 2_i32, var_both)
  } /*block_1: single*/

fn t_6single_acos(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  OpMathFuncSingle(stores, 3_i32, var_both)
  } /*block_1: single*/

fn t_6single_asin(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  OpMathFuncSingle(stores, 4_i32, var_both)
  } /*block_1: single*/

fn t_6single_atan(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  OpMathFuncSingle(stores, 5_i32, var_both)
  } /*block_1: single*/

fn t_6single_ceil(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  OpMathFuncSingle(stores, 6_i32, var_both)
  } /*block_1: single*/

fn t_6single_floor(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  OpMathFuncSingle(stores, 7_i32, var_both)
  } /*block_1: single*/

fn t_6single_round(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  OpMathFuncSingle(stores, 8_i32, var_both)
  } /*block_1: single*/

fn t_6single_sqrt(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  OpMathFuncSingle(stores, 9_i32, var_both)
  } /*block_1: single*/

fn OpMathFunc2Single(stores: &mut Stores, mut var_fn_id: i8, mut var_v1: f32, mut var_v2: f32) -> f32 {

}


fn t_6single_atan2(stores: &mut Stores, mut var_both: f32, mut var_v2: f32) -> f32 { //block_1: single
  OpMathFunc2Single(stores, 0_i32, var_both, var_v2)
  } /*block_1: single*/

fn t_6single_log(stores: &mut Stores, mut var_both: f32, mut var_v2: f32) -> f32 { //block_1: single
  OpMathFunc2Single(stores, 1_i32, var_both, var_v2)
  } /*block_1: single*/

fn t_6single_pow(stores: &mut Stores, mut var_both: f32, mut var_v2: f32) -> f32 { //block_1: single
  (var_both).powf((var_v2))
  } /*block_1: single*/

fn OpFormatSingle(stores: &mut Stores, mut var_pos: u16, mut var_val: f32, mut var_width: i32, mut var_precision: i32) {

}


fn OpFormatStackSingle(stores: &mut Stores, mut var_pos: u16, mut var_val: f32, mut var_width: i32, mut var_precision: i32) {

}


fn t_5float_abs(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  (var_both).abs()
  } /*block_1: float*/

fn OpMathFuncFloat(stores: &mut Stores, mut var_fn_id: i8, mut var_v1: f64) -> f64 {

}


fn t_5float_cos(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  OpMathFuncFloat(stores, 0_i32, var_both)
  } /*block_1: float*/

fn t_5float_sin(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  OpMathFuncFloat(stores, 1_i32, var_both)
  } /*block_1: float*/

fn t_5float_tan(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  OpMathFuncFloat(stores, 2_i32, var_both)
  } /*block_1: float*/

fn t_5float_acos(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  OpMathFuncFloat(stores, 3_i32, var_both)
  } /*block_1: float*/

fn t_5float_asin(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  OpMathFuncFloat(stores, 4_i32, var_both)
  } /*block_1: float*/

fn t_5float_atan(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  OpMathFuncFloat(stores, 5_i32, var_both)
  } /*block_1: float*/

fn t_5float_ceil(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  OpMathFuncFloat(stores, 6_i32, var_both)
  } /*block_1: float*/

fn t_5float_floor(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  OpMathFuncFloat(stores, 7_i32, var_both)
  } /*block_1: float*/

fn t_5float_round(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  OpMathFuncFloat(stores, 8_i32, var_both)
  } /*block_1: float*/

fn t_5float_sqrt(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  OpMathFuncFloat(stores, 9_i32, var_both)
  } /*block_1: float*/

fn OpMathFunc2Float(stores: &mut Stores, mut var_fn_id: i8, mut var_v1: f64, mut var_v2: f64) -> f64 {

}


fn t_5float_atan2(stores: &mut Stores, mut var_both: f64, mut var_v2: f64) -> f64 { //block_1: float
  OpMathFunc2Float(stores, 0_i32, var_both, var_v2)
  } /*block_1: float*/

fn t_5float_log(stores: &mut Stores, mut var_both: f64, mut var_v2: f64) -> f64 { //block_1: float
  OpMathFunc2Float(stores, 1_i32, var_both, var_v2)
  } /*block_1: float*/

fn t_5float_pow(stores: &mut Stores, mut var_both: f64, mut var_v2: f64) -> f64 { //block_1: float
  (var_both).powf((var_v2))
  } /*block_1: float*/

fn t_6single_exp(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  t_6single_pow(stores, (std::f64::consts::E) as f32, var_both)
  } /*block_1: single*/

fn t_5float_exp(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  t_5float_pow(stores, std::f64::consts::E, var_both)
  } /*block_1: float*/

fn t_6single_ln(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  t_6single_log(stores, var_both, (std::f64::consts::E) as f32)
  } /*block_1: single*/

fn t_5float_ln(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  t_5float_log(stores, var_both, std::f64::consts::E)
  } /*block_1: float*/

fn t_6single_log2(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  t_6single_log(stores, var_both, 2_f32)
  } /*block_1: single*/

fn t_5float_log2(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  t_5float_log(stores, var_both, 2_f64)
  } /*block_1: float*/

fn t_6single_log10(stores: &mut Stores, mut var_both: f32) -> f32 { //block_1: single
  t_6single_log(stores, var_both, 10_f32)
  } /*block_1: single*/

fn t_5float_log10(stores: &mut Stores, mut var_both: f64) -> f64 { //block_1: float
  t_5float_log(stores, var_both, 10_f64)
  } /*block_1: float*/

fn OpFormatFloat(stores: &mut Stores, mut var_pos: u16, mut var_val: f64, mut var_width: i32, mut var_precision: i32) {

}


fn OpFormatStackFloat(stores: &mut Stores, mut var_pos: u16, mut var_val: f64, mut var_width: i32, mut var_precision: i32) {

}


fn t_7integer_min(stores: &mut Stores, mut var_both: i32, mut var_b: i32) -> i32 { //block_1: integer
  if if !(external::op_conv_bool_from_int((var_both))) {true} else {!(external::op_conv_bool_from_int((var_b)))} { //block_2: void
    return i32::MIN;
    } /*block_2: void*/ else {()};
  if (var_both) <= (var_b) { //block_3: integer
    var_both
    } /*block_3: integer*/ else { //block_4: integer
    var_b
    } /*block_4: integer*/
  } /*block_1: integer*/

fn t_7integer_max(stores: &mut Stores, mut var_both: i32, mut var_b: i32) -> i32 { //block_1: integer
  if if !(external::op_conv_bool_from_int((var_both))) {true} else {!(external::op_conv_bool_from_int((var_b)))} { //block_2: void
    return i32::MIN;
    } /*block_2: void*/ else {()};
  if (var_b) <= (var_both) { //block_3: integer
    var_both
    } /*block_3: integer*/ else { //block_4: integer
    var_b
    } /*block_4: integer*/
  } /*block_1: integer*/

fn t_7integer_clamp(stores: &mut Stores, mut var_both: i32, mut var_lo: i32, mut var_hi: i32) -> i32 { //block_1: integer
  let _pre0 = t_7integer_max(stores, var_both, var_lo);
  t_7integer_min(stores, _pre0, var_hi)
  } /*block_1: integer*/

fn t_4long_min(stores: &mut Stores, mut var_both: i64, mut var_b: i64) -> i64 { //block_1: long
  if if !(external::op_conv_bool_from_long((var_both))) {true} else {!(external::op_conv_bool_from_long((var_b)))} { //block_2: void
    return i64::MIN;
    } /*block_2: void*/ else {()};
  if (var_both) <= (var_b) { //block_3: long
    var_both
    } /*block_3: long*/ else { //block_4: long
    var_b
    } /*block_4: long*/
  } /*block_1: long*/

fn t_4long_max(stores: &mut Stores, mut var_both: i64, mut var_b: i64) -> i64 { //block_1: long
  if if !(external::op_conv_bool_from_long((var_both))) {true} else {!(external::op_conv_bool_from_long((var_b)))} { //block_2: void
    return i64::MIN;
    } /*block_2: void*/ else {()};
  if (var_b) <= (var_both) { //block_3: long
    var_both
    } /*block_3: long*/ else { //block_4: long
    var_b
    } /*block_4: long*/
  } /*block_1: long*/

fn t_4long_clamp(stores: &mut Stores, mut var_both: i64, mut var_lo: i64, mut var_hi: i64) -> i64 { //block_1: long
  let _pre1 = t_4long_max(stores, var_both, var_lo);
  t_4long_min(stores, _pre1, var_hi)
  } /*block_1: long*/

fn t_6single_min(stores: &mut Stores, mut var_both: f32, mut var_b: f32) -> f32 { //block_1: single
  if if !(!(var_both).is_nan()) {true} else {!(!(var_b).is_nan())} { //block_2: void
    return f32::NAN;
    } /*block_2: void*/ else {()};
  if (var_both) <= (var_b) { //block_3: single
    var_both
    } /*block_3: single*/ else { //block_4: single
    var_b
    } /*block_4: single*/
  } /*block_1: single*/

fn t_6single_max(stores: &mut Stores, mut var_both: f32, mut var_b: f32) -> f32 { //block_1: single
  if if !(!(var_both).is_nan()) {true} else {!(!(var_b).is_nan())} { //block_2: void
    return f32::NAN;
    } /*block_2: void*/ else {()};
  if (var_b) <= (var_both) { //block_3: single
    var_both
    } /*block_3: single*/ else { //block_4: single
    var_b
    } /*block_4: single*/
  } /*block_1: single*/

fn t_6single_clamp(stores: &mut Stores, mut var_both: f32, mut var_lo: f32, mut var_hi: f32) -> f32 { //block_1: single
  let _pre2 = t_6single_max(stores, var_both, var_lo);
  t_6single_min(stores, _pre2, var_hi)
  } /*block_1: single*/

fn t_5float_min(stores: &mut Stores, mut var_both: f64, mut var_b: f64) -> f64 { //block_1: float
  if if !(!(var_both).is_nan()) {true} else {!(!(var_b).is_nan())} { //block_2: void
    return f64::NAN;
    } /*block_2: void*/ else {()};
  if (var_both) <= (var_b) { //block_3: float
    var_both
    } /*block_3: float*/ else { //block_4: float
    var_b
    } /*block_4: float*/
  } /*block_1: float*/

fn t_5float_max(stores: &mut Stores, mut var_both: f64, mut var_b: f64) -> f64 { //block_1: float
  if if !(!(var_both).is_nan()) {true} else {!(!(var_b).is_nan())} { //block_2: void
    return f64::NAN;
    } /*block_2: void*/ else {()};
  if (var_b) <= (var_both) { //block_3: float
    var_both
    } /*block_3: float*/ else { //block_4: float
    var_b
    } /*block_4: float*/
  } /*block_1: float*/

fn t_5float_clamp(stores: &mut Stores, mut var_both: f64, mut var_lo: f64, mut var_hi: f64) -> f64 { //block_1: float
  let _pre3 = t_5float_max(stores, var_both, var_lo);
  t_5float_min(stores, _pre3, var_hi)
  } /*block_1: float*/

fn OpVarText(stores: &mut Stores, mut var_pos: u16) -> Str {

}


fn OpArgText(stores: &mut Stores, mut var_pos: u16) -> Str {

}


fn OpConvTextFromNull(stores: &mut Stores) -> Str {

}


fn t_4text_len(stores: &mut Stores, mut var_both: Str) -> i32 { //block_1: integer
  (&var_both).len() as i32
  } /*block_1: integer*/

fn OpLengthCharacter(stores: &mut Stores, mut var_v1: i32) -> i32 {

}


fn t_9character_len(stores: &mut Stores, mut var_both: i32) -> i32 { //block_1: integer
  OpLengthCharacter(stores, var_both)
  } /*block_1: integer*/

fn OpText(stores: &mut Stores) {

}


fn OpAppendText(stores: &mut Stores, mut var_pos: u16, mut var_v1: Str) {

}


fn OpGetTextSub(stores: &mut Stores, mut var_v1: Str, mut var_from: i32, mut var_till: i32) -> Str {

}


fn OpClearText(stores: &mut Stores, mut var_pos: u16) {

}


fn OpFreeText(stores: &mut Stores, mut var_pos: u16) {

}


fn OpFormatText(stores: &mut Stores, mut var_pos: u16, mut var_val: Str, mut var_width: i32, mut var_dir: i8, mut var_token: u8) {

}


fn OpFormatStackText(stores: &mut Stores, mut var_pos: u16, mut var_val: Str, mut var_width: i32, mut var_dir: i8, mut var_token: u8) {

}


fn OpAppendCharacter(stores: &mut Stores, mut var_pos: u16, mut var_v1: i32) {

}


fn OpTextCompare(stores: &mut Stores, mut var_v1: Str, mut var_v2: i32) -> i32 {

}


fn OpDatabase(stores: &mut Stores, mut var_pos: u16, mut var_db_tp: u16) {

}


fn OpFormatDatabase(stores: &mut Stores, mut var_pos: u16, mut var_val: DbRef, mut var_db_tp: u16, mut var_db_format: u8) {

}


fn OpFormatStackDatabase(stores: &mut Stores, mut var_pos: u16, mut var_val: DbRef, mut var_db_tp: u16, mut var_db_format: u8) {

}


fn OpFreeRef(stores: &mut Stores, mut var_v1: DbRef) {

}


fn OpSizeofRef(stores: &mut Stores, mut var_val: DbRef) -> i32 {

}


fn t_6vector_len(stores: &mut Stores, mut var_both: DbRef) -> i32 { //block_1: integer
  vector::length_vector(&(var_both), &s.database.allocations) as i32
  } /*block_1: integer*/

fn OpInsertVector(stores: &mut Stores, mut var_r: DbRef, mut var_size: u16, mut var_index: i32, mut var_db_tp: u16) -> DbRef {

}


fn OpNewRecord(stores: &mut Stores, mut var_data: DbRef, mut var_parent_tp: u16, mut var_fld: u16) -> DbRef {

}


fn OpFinishRecord(stores: &mut Stores, mut var_data: DbRef, mut var_rec: DbRef, mut var_parent_tp: u16, mut var_fld: u16) {

}


fn OpGetRecord(stores: &mut Stores, mut var_data: DbRef, mut var_db_tp: u16, mut var_no_keys: u8) -> DbRef {

}


fn OpValidate(stores: &mut Stores, mut var_data: DbRef, mut var_db_tp: u16) {

}


fn OpHashAdd(stores: &mut Stores, mut var_data: DbRef, mut var_rec: DbRef, mut var_tp: u16) {

}


fn OpHashFind(stores: &mut Stores, mut var_data: DbRef, mut var_tp: u16) -> DbRef {

}


fn OpHashRemove(stores: &mut Stores, mut var_data: DbRef, mut var_rec: DbRef, mut var_tp: u16) {

}


fn n_assert(stores: &mut Stores, mut var_test: bool, mut var_message: Str, mut var_file: Str, mut var_line: i32) {

}


fn n_panic(stores: &mut Stores, mut var_message: Str, mut var_file: Str, mut var_line: i32) {

}


fn n_log_info(stores: &mut Stores, mut var_message: Str, mut var_file: Str, mut var_line: i32) {

}


fn n_log_warn(stores: &mut Stores, mut var_message: Str, mut var_file: Str, mut var_line: i32) {

}


fn n_log_error(stores: &mut Stores, mut var_message: Str, mut var_file: Str, mut var_line: i32) {

}


fn n_log_fatal(stores: &mut Stores, mut var_message: Str, mut var_file: Str, mut var_line: i32) {

}


fn n_print(stores: &mut Stores, mut var_v1: Str) { //block_1: void
  print!("{}", (&var_v1));;
  } /*block_1: void*/

fn n_println(stores: &mut Stores, mut var_v1: Str) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let _pre4 = { //Formatted string_2: text["__work_1"]
    var___work_1 = "".to_string();
    ops::format_text(&mut var___work_1, &var_v1, 0_i32, -1, 32);
    var___work_1 += "\n";
    &var___work_1
    } /*Formatted string_2: text["__work_1"]*/;
  print!("{}", (_pre4));;
  ;
  } /*block_1: void*/

fn OpIterate(stores: &mut Stores, mut var_data: DbRef, mut var_on: u8, mut var_arg: u16, mut var_keys: &[Key], mut var_from_key: u8, mut var_till_key: u8) -> i64 {

}


fn OpStep(stores: &mut Stores, mut var_state_var: u16, mut var_data: DbRef, mut var_on: u8, mut var_arg: u16) -> DbRef {

}


fn OpRemove(stores: &mut Stores, mut var_state_var: u16, mut var_data: DbRef, mut var_on: u8, mut var_tp: u16) {

}


fn OpClear(stores: &mut Stores, mut var_data: DbRef, mut var_tp: u16) {

}


fn OpAppendCopy(stores: &mut Stores, mut var_data: DbRef, mut var_count: i32, mut var_tp: u16) {

}


fn OpCopyRecord(stores: &mut Stores, mut var_data: DbRef, mut var_to: DbRef, mut var_tp: u16) {

}


fn OpStaticCall(stores: &mut Stores, mut var_call: u16) {

}


fn OpCreateStack(stores: &mut Stores, mut var_pos: u16) -> DbRef {

}


fn OpGetStackText(stores: &mut Stores, mut var_r: DbRef) -> Str {

}


fn OpGetStackRef(stores: &mut Stores, mut var_r: DbRef, mut var_fld: u16) -> DbRef {

}


fn OpSetStackRef(stores: &mut Stores, mut var_r: DbRef, mut var_v1: DbRef) {

}


fn OpAppendStackText(stores: &mut Stores, mut var_pos: u16, mut var_v1: Str) {

}


fn OpAppendStackCharacter(stores: &mut Stores, mut var_pos: u16, mut var_v1: i32) {

}


fn OpClearStackText(stores: &mut Stores, mut var_pos: u16) {

}


fn n_get_store_lock(stores: &mut Stores, mut var_r: DbRef) -> bool {

}


fn n_set_store_lock(stores: &mut Stores, mut var_r: DbRef, mut var_locked: bool) {

}


fn n_parallel_for_int(stores: &mut Stores, mut var_func: Str, mut var_input: DbRef, mut var_element_size: i32, mut var_threads: i32) -> DbRef {

}


fn n_parallel_for(stores: &mut Stores, mut var_input: DbRef, mut var_element_size: i32, mut var_return_size: i32, mut var_threads: i32, mut var_func: i32) -> DbRef {

}


fn n_parallel_get_int(stores: &mut Stores, mut var_r: DbRef, mut var_idx: i32) -> i32 {

}


fn n_parallel_get_long(stores: &mut Stores, mut var_r: DbRef, mut var_idx: i32) -> i64 {

}


fn n_parallel_get_float(stores: &mut Stores, mut var_r: DbRef, mut var_idx: i32) -> f64 {

}


fn n_parallel_get_bool(stores: &mut Stores, mut var_r: DbRef, mut var_idx: i32) -> bool {

}


fn OpGetFileText(stores: &mut Stores, mut var_file: DbRef, mut var_content: &mut String) {

}


fn OpWriteFile(stores: &mut Stores, mut var_file: DbRef, mut var_val: DbRef, mut var_db_tp: u16) {

}


fn OpReadFile(stores: &mut Stores, mut var_file: DbRef, mut var_val: DbRef, mut var_bytes: i32, mut var_db_tp: u16) {

}


fn OpSeekFile(stores: &mut Stores, mut var_file: DbRef, mut var_pos: i64) {

}


fn OpSizeFile(stores: &mut Stores, mut var_file: DbRef) -> i64 {

}


fn OpTruncateFile(stores: &mut Stores, mut var_self: DbRef, mut var_size: i64) -> bool {

}


fn t_5Pixel_value(stores: &mut Stores, mut var_self: DbRef) -> i32 { //block_1: integer
  external::op_add_int((external::op_add_int((external::op_mul_int(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), i32::from((0_i32)))}), (65536_i32))), (external::op_mul_int(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((1_i32)), i32::from((0_i32)))}), (256_i32))))), ({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((2_i32)), i32::from((0_i32)))}))
  } /*block_1: integer*/

fn t_4File_content(stores: &mut Stores, mut var_self: DbRef, mut var_result: &mut String) -> Str { //block_1: text["result"]
  *var_result = "".to_string();
  let mut var_txt: String = "".to_string();
  let _pre5 = ;
  _pre5O_pre5p_pre5G_pre5e_pre5t_pre5F_pre5i_pre5l_pre5e_pre5T_pre5e_pre5x_pre5t_pre5(_pre5s_pre5t_pre5o_pre5r_pre5e_pre5s_pre5,_pre5 _pre5v_pre5a_pre5r_pre5__pre5s_pre5e_pre5l_pre5f_pre5,_pre5 _pre5)_pre5;
  *var_result += &var_txt;
  ;
  Str::new(var_result)
  } /*block_1: text["result"]*/

fn t_4File_lines(stores: &mut Stores, mut var_self: DbRef, mut var_result: DbRef) -> DbRef { //block_1: vector<text>["result"]
  let mut var___work_1: String = "".to_string();
  vector::clear_vector(&(var_result), &mut s.database.allocations);;
  let mut var_c: String = t_4File_content(stores, var_self, &mut var___work_1).to_string();
  let mut var_p: i32 = 0_i32;
  { //For block_3: void
    let mut var_ch__index: i32 = 0_i32;
    let mut var_ch__next: i32 = 0_i32;
    loop { //For loop_4
      let mut var_ch: i32 = { //for text next_5: character
        var_ch__index = var_ch__next;
        let mut var__for_result_1: i32 = external::text_character((&var_c), (var_ch__next));
        var_ch__next = external::op_add_int((var_ch__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_5: character*/;
      if !(external::op_conv_bool_from_character((var_ch))) { //break_6: void
        break;
        } /*break_6: void*/ else {()};
      { //block_7: void
        let mut var__elm_2: DbRef = stores.null();
        if (if (var_ch) == char::from(0) { i32::MIN } else { (var_ch) as i32 }) == (if (char::from_u32(10_u32).unwrap_or('\0')) == char::from(0) { i32::MIN } else { (char::from_u32(10_u32).unwrap_or('\0')) as i32 }) { //block_8: void
          var__elm_2 = OpNewRecord(stores, var_result, 7_i32, 65535_i32);
          let _pre6 = OpGetTextSub(stores, &var_c, var_p, var_ch__index);
          {let db = (var__elm_2); let s_val = (_pre6).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
          OpFinishRecord(stores, var_result, var__elm_2, 7_i32, 65535_i32);
          var_p = var_ch__next;
          } /*block_8: void*/ else {()};
        } /*block_7: void*/;
      } /*For loop_4*/;
    } /*For block_3: void*/;
  let mut var__elm_3: DbRef = stores.null();
  if (0_i32) < (var_p) { //block_9: void
    var__elm_3 = OpNewRecord(stores, var_result, 7_i32, 65535_i32);
    let _pre8 = t_4text_len(stores, &var_c);
    let _pre7 = OpGetTextSub(stores, &var_c, var_p, _pre8);
    {let db = (var__elm_3); let s_val = (_pre7).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    OpFinishRecord(stores, var_result, var__elm_3, 7_i32, 65535_i32);
    } /*block_9: void*/ else {()};
  ;
  ;
  var_result
  } /*block_1: vector<text>["result"]*/

fn t_4text_split(stores: &mut Stores, mut var_self: Str, mut var_separator: i32, mut var_result: DbRef) -> DbRef { //block_1: vector<text>["result"]
  vector::clear_vector(&(var_result), &mut s.database.allocations);;
  let mut var_p: i32 = 0_i32;
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = external::text_character((&var_self), (var_c__next));
        var_c__next = external::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(external::op_conv_bool_from_character((var_c))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        let mut var__elm_2: DbRef = stores.null();
        if (if (var_c) == char::from(0) { i32::MIN } else { (var_c) as i32 }) == (if (var_separator) == char::from(0) { i32::MIN } else { (var_separator) as i32 }) { //block_7: void
          var__elm_2 = OpNewRecord(stores, var_result, 7_i32, 65535_i32);
          let _pre9 = OpGetTextSub(stores, &var_self, var_p, var_c__index);
          {let db = (var__elm_2); let s_val = (_pre9).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
          OpFinishRecord(stores, var_result, var__elm_2, 7_i32, 65535_i32);
          var_p = var_c__next;
          } /*block_7: void*/ else {()};
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  let mut var__elm_3: DbRef = stores.null();
  if (0_i32) < (var_p) { //block_8: void
    var__elm_3 = OpNewRecord(stores, var_result, 7_i32, 65535_i32);
    let _pre11 = t_4text_len(stores, &var_self);
    let _pre10 = OpGetTextSub(stores, &var_self, var_p, _pre11);
    {let db = (var__elm_3); let s_val = (_pre10).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    OpFinishRecord(stores, var_result, var__elm_3, 7_i32, 65535_i32);
    } /*block_8: void*/ else {()};
  var_result
  } /*block_1: vector<text>["result"]*/

fn n_valid_path(stores: &mut Stores, mut var_path: Str) -> bool { //block_1: boolean
  let mut var_depth: i32 = 0_i32;
  let mut var_start: i32 = 0_i32;
  { //For block_2: void
    let mut var_c__index: i32 = 0_i32;
    let mut var_c__next: i32 = 0_i32;
    loop { //For loop_3
      let mut var_c: i32 = { //for text next_4: character
        var_c__index = var_c__next;
        let mut var__for_result_1: i32 = external::text_character((&var_path), (var_c__next));
        var_c__next = external::op_add_int((var_c__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(external::op_conv_bool_from_character((var_c))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        let mut var_part: String = "".to_string();
        if if (if (var_c) == char::from(0) { i32::MIN } else { (var_c) as i32 }) == (if (char::from_u32(47_u32).unwrap_or('\0')) == char::from(0) { i32::MIN } else { (char::from_u32(47_u32).unwrap_or('\0')) as i32 }) {true} else {(if (var_c) == char::from(0) { i32::MIN } else { (var_c) as i32 }) == (if (char::from_u32(92_u32).unwrap_or('\0')) == char::from(0) { i32::MIN } else { (char::from_u32(92_u32).unwrap_or('\0')) as i32 })} { //block_7: void
          var_part = OpGetTextSub(stores, &var_path, var_start, var_c__index).to_string();
          var_start = var_c__next;
          if (&var_part) == ("..") { //block_8: void
            var_depth = external::op_min_int((var_depth), (1_i32));
            if (var_depth) < (0_i32) { //block_9: void
              false;
              ;
              return ();
              } /*block_9: void*/ else {()};
            } /*block_8: void*/ else {if if (&var_part) != (".") {(0_i32) < (t_4text_len(stores, &var_part))} else {false} { //block_10: void
              var_depth = external::op_add_int((var_depth), (1_i32));
              } /*block_10: void*/ else {()}};
          } /*block_7: void*/ else {()};
        ;
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  if (var_start) < (t_4text_len(stores, &var_path)) { //block_11: void
    let mut var_part: String = OpGetTextSub(stores, &var_path, var_start, t_4text_len(stores, &var_path)).to_string();
    if (&var_part) == ("..") { //block_12: void
      var_depth = external::op_min_int((var_depth), (1_i32));
      } /*block_12: void*/ else {()};
    ;
    } /*block_11: void*/ else {()};
  (0_i32) <= (var_depth)
  } /*block_1: boolean*/

fn n_file(stores: &mut Stores, mut var_path: Str, mut var_result: DbRef) -> DbRef { //block_1: ref(File)["result"]
  OpDatabase(stores, var_result, 14_i32);
  {let db = (var_result); let s_val = (&var_path).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((24_i32)), s_pos as i32);};
  {let db = (var_result); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((28_i32)), (i32::MIN));};
  {let db = (var_result); stores.store_mut(&db).set_long(db.rec, db.pos + u32::from((0_i32)), (0_i64));};
  {let db = (var_result); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((32_i32)), 0, i32::from((0_u8)));};
  {let db = (var_result); stores.store_mut(&db).set_long(db.rec, db.pos + u32::from((8_i32)), (0_i64));};
  {let db = (var_result); stores.store_mut(&db).set_long(db.rec, db.pos + u32::from((16_i32)), (0_i64));};
  if n_valid_path(stores, &var_path) { //block_2: void
    stores.get_file(&(var_result));
    } /*block_2: void*/ else { //block_3: void
    {let db = (var_result); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((32_i32)), 0, i32::from((5_u8)));};
    } /*block_3: void*/;
  var_result
  } /*block_1: ref(File)["result"]*/

fn n_exists(stores: &mut Stores, mut var_path: Str) -> bool { //block_1: boolean
  let mut var___ref_1: DbRef = stores.null();
  let _pre12 = n_file(stores, &var_path, var___ref_1);
  let _ret = (if ({let db = (_pre12); stores.store(&db).get_byte(db.rec, db.pos + u32::from((32_i32)), 0) as u8}) == 255 { i32::MIN } else { i32::from(({let db = (_pre12); stores.store(&db).get_byte(db.rec, db.pos + u32::from((32_i32)), 0) as u8})) }) != (if (5_u8) == 255 { i32::MIN } else { i32::from((5_u8)) });
  OpFreeRef(stores, var___ref_1);
  _ret
  } /*block_1: boolean*/

fn n_delete(stores: &mut Stores, mut var_path: Str) -> bool { //block_1: boolean
  if n_exists(stores, &var_path) {std::fs::remove_file((&var_path)).is_ok()} else {false}
  } /*block_1: boolean*/

fn n_move(stores: &mut Stores, mut var_from: Str, mut var_to: Str) -> bool { //block_1: boolean
  if if if n_valid_path(stores, &var_to) {n_exists(stores, &var_from)} else {false} {!(n_exists(stores, &var_to))} else {false} {std::fs::rename((&var_from), (&var_to)).is_ok()} else {false}
  } /*block_1: boolean*/

fn t_4File_set_file_size(stores: &mut Stores, mut var_self: DbRef, mut var_size: i64) -> bool { //block_1: boolean
  if if if (if ({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((32_i32)), 0) as u8}) == 255 { i32::MIN } else { i32::from(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((32_i32)), 0) as u8})) }) != (if (4_u8) == 255 { i32::MIN } else { i32::from((4_u8)) }) {(if ({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((32_i32)), 0) as u8}) == 255 { i32::MIN } else { i32::from(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((32_i32)), 0) as u8})) }) != (if (5_u8) == 255 { i32::MIN } else { i32::from((5_u8)) })} else {false} {(0_i64) <= (var_size)} else {false} {OpTruncateFile(stores, var_self, var_size)} else {false}
  } /*block_1: boolean*/

fn t_4File_files(stores: &mut Stores, mut var_self: DbRef, mut var_result: DbRef) -> DbRef { //block_1: vector<ref(File)>["result"]
  vector::clear_vector(&(var_result), &mut s.database.allocations);;
  if (if ({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((32_i32)), 0) as u8}) == 255 { i32::MIN } else { i32::from(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((32_i32)), 0) as u8})) }) == (if (4_u8) == 255 { i32::MIN } else { i32::from((4_u8)) }) { //block_2: void
    stores.get_dir(({let db = (var_self); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((24_i32))) as u32))}), &(var_result));
    } /*block_2: void*/ else {()};
  var_result
  } /*block_1: vector<ref(File)>["result"]*/

fn t_4File_write(stores: &mut Stores, mut var_self: DbRef, mut var_v: Str) {

}


fn t_4File_png(stores: &mut Stores, mut var_self: DbRef, mut var_result: DbRef) -> DbRef { //block_1: ref(Image)["result"]
  if (if ({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((32_i32)), 0) as u8}) == 255 { i32::MIN } else { i32::from(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((32_i32)), 0) as u8})) }) == (if (1_u8) == 255 { i32::MIN } else { i32::from((1_u8)) }) { //block_2: ref(Image)["result"]
    OpDatabase(stores, var_result, 12_i32);
    {let db = (var_result); let s_val = ("").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    {let db = (var_result); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (0_i32));};
    {let db = (var_result); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((8_i32)), (0_i32));};
    {let db = (var_result); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((12_i32)), (0_i32));};
    stores.get_png(({let db = (var_self); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((24_i32))) as u32))}), &(var_result));
    var_result
    } /*block_2: ref(Image)["result"]*/ else { //block_3: ref(Image)["result"]
    stores.null()
    } /*block_3: ref(Image)["result"]*/
  } /*block_1: ref(Image)["result"]*/

fn n_env_variables(stores: &mut Stores) -> DbRef {

}


fn n_env_variable(stores: &mut Stores, mut var_name: Str) -> Str {

}


fn n_rand(stores: &mut Stores, mut var_lo: i32, mut var_hi: i32) -> i32 {

}


fn n_rand_seed(stores: &mut Stores, mut var_seed: i64) {

}


fn n_rand_indices(stores: &mut Stores, mut var_n: i32) -> DbRef {

}


fn n_now(stores: &mut Stores) -> i64 {

}


fn n_ticks(stores: &mut Stores) -> i64 {

}


fn t_4text_starts_with(stores: &mut Stores, mut var_self: Str, mut var_value: Str) -> bool {

}


fn t_4text_ends_with(stores: &mut Stores, mut var_self: Str, mut var_value: Str) -> bool {

}


fn t_4text_trim(stores: &mut Stores, mut var_both: Str) -> Str {

}


fn t_4text_trim_start(stores: &mut Stores, mut var_self: Str) -> Str {

}


fn t_4text_trim_end(stores: &mut Stores, mut var_self: Str) -> Str {

}


fn t_4text_find(stores: &mut Stores, mut var_self: Str, mut var_value: Str) -> i32 {

}


fn t_4text_rfind(stores: &mut Stores, mut var_self: Str, mut var_value: Str) -> i32 {

}


fn t_4text_contains(stores: &mut Stores, mut var_self: Str, mut var_value: Str) -> bool {

}


fn t_4text_replace(stores: &mut Stores, mut var_self: Str, mut var_value: Str, mut var_with: Str) -> Str {

}


fn t_4text_to_lowercase(stores: &mut Stores, mut var_self: Str) -> Str {

}


fn t_4text_to_uppercase(stores: &mut Stores, mut var_self: Str) -> Str {

}


fn t_4text_is_lowercase(stores: &mut Stores, mut var_self: Str) -> bool {

}


fn t_9character_is_lowercase(stores: &mut Stores, mut var_self: i32) -> bool {

}


fn t_4text_is_uppercase(stores: &mut Stores, mut var_self: Str) -> bool {

}


fn t_9character_is_uppercase(stores: &mut Stores, mut var_self: i32) -> bool {

}


fn t_4text_is_numeric(stores: &mut Stores, mut var_self: Str) -> bool {

}


fn t_9character_is_numeric(stores: &mut Stores, mut var_self: i32) -> bool {

}


fn t_4text_is_alphanumeric(stores: &mut Stores, mut var_self: Str) -> bool {

}


fn t_9character_is_alphanumeric(stores: &mut Stores, mut var_self: i32) -> bool {

}


fn t_4text_is_alphabetic(stores: &mut Stores, mut var_self: Str) -> bool {

}


fn t_9character_is_alphabetic(stores: &mut Stores, mut var_self: i32) -> bool {

}


fn t_4text_is_whitespace(stores: &mut Stores, mut var_self: Str) -> bool {

}


fn t_4text_is_control(stores: &mut Stores, mut var_self: Str) -> bool {

}


fn n_join(stores: &mut Stores, mut var_parts: DbRef, mut var_sep: Str, mut var_result: &mut String) -> Str { //block_1: text["result"]
  *var_result = "".to_string();
  { //For block_2: void
    let mut var_p__count: i32 = 0_i32;
    let mut var__vector_1: DbRef = var_parts;
    let mut var_p__index: i32 = -1_i32;
    loop { //For loop_3
      let mut var_p = { //iter next_4: text
        var_p__index = external::op_add_int((var_p__index), (1_i32));
        {let db = (vector::get_vector(&(var__vector_1), u32::from((4_i32)), (var_p__index), &s.database.allocations)); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((0_i32))) as u32))}
        } /*iter next_4: text*/.to_string();
      if !((&var_p) != crate::state::STRING_NULL) { //break_5: void
        ;
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        if !((var_p__count) == (0_i32)) { //block_7: void
          *var_result += &var_sep;
          } /*block_7: void*/ else {()};
        *var_result += &var_p;
        } /*block_6: void*/;
      var_p__count = external::op_add_int((var_p__count), (1_i32));
      ;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  Str::new(var_result)
  } /*block_1: text["result"]*/

fn n_arguments(stores: &mut Stores) -> DbRef {

}


fn n_directory(stores: &mut Stores, mut var_v: &mut String) -> Str {

}


fn n_user_directory(stores: &mut Stores, mut var_v: &mut String) -> Str {

}


fn n_program_directory(stores: &mut Stores, mut var_v: &mut String) -> Str {

}


