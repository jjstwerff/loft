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
    db.finish();
}

fn n_routine(stores: &mut Stores, mut var_a: i32, mut var___work_1: &mut String) -> Str { //block_1: text["__work_1"]
  *var___work_1 = "".to_string();
  if (2_i32) < (var_a) { //block_2: void
    return Str::new(OpConvTextFromNull(stores));
    } /*block_2: void*/ else {()};
  Str::new({ //Formatted string_3: text["__work_1"]
    *var___work_1 = "#".to_string();
    OpFormatStackLong(stores, var___work_1, external::op_conv_long_from_int((var_a)), 10_i32, 0_i32, 32_i32, false, false);
    *var___work_1 += "#";
    var___work_1
    } /*Formatted string_3: text["__work_1"]*/)
  } /*block_1: text["__work_1"]*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text
    n_routine(stores, 5_i32, &mut var___work_1)
    } /*block_2: text*/.to_string();
  let _pre13 = (&var_test_value) == (OpConvTextFromNull(stores));
  let _pre14 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != null";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, _pre13, _pre14, "call_text_null", 6_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_call_text_null() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
