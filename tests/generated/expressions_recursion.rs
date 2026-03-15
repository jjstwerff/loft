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
    let e = db.enumerate("State");
    db.value(e, "Start", u16::MAX);
    db.value(e, "Ongoing", u16::MAX);
    db.value(e, "Halt", u16::MAX);
    db.finish();
}

fn n_first(stores: &mut Stores, mut var_s: u8, mut var_c: i32) -> i32 { //block_1: integer
  if (if (var_s) == 255 { i32::MIN } else { i32::from((var_s)) }) == (if (1_u8) == 255 { i32::MIN } else { i32::from((1_u8)) }) { //block_2: void
    let mut var_s: u8 = 2_u8;
    } /*block_2: void*/ else {if (10_i32) < (var_c) { //block_3: void
      var_s = 3_u8;
      } /*block_3: void*/ else {()}};
  n_second(stores, var_s, var_c)
  } /*block_1: integer*/

fn n_second(stores: &mut Stores, mut var_s: u8, mut var_c: i32) -> i32 { //block_1: integer
  if (if (var_s) == 255 { i32::MIN } else { i32::from((var_s)) }) != (if (3_u8) == 255 { i32::MIN } else { i32::from((3_u8)) }) { //block_2: integer
    n_first(stores, var_s, external::op_add_int((var_c), (1_i32)))
    } /*block_2: integer*/ else { //block_3: integer
    external::op_add_int((1_i32), (var_c))
    } /*block_3: integer*/
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    n_first(stores, 1_u8, 0_i32)
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 12";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (12_i32), _pre13, "recursion", 27_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_recursion() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
