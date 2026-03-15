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

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    external::op_div_int((2_i32), (external::op_min_int((external::op_min_int((3_i32), (2_i32))), (1_i32))))
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != null";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (i32::MIN), _pre13, "expr_zero_divide", 4_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_expr_zero_divide() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
