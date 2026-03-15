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

fn n_routine(stores: &mut Stores) -> i32 { //block_1: integer
  let mut var_b: i32 = 0_i32;
  { //For block_2: void
    let mut var_a__index: i32 = i32::MIN;
    loop { //For loop_3
      let mut var_a: i32 = { //Iter range_4: integer
        var_a__index = if !(external::op_conv_bool_from_int((var_a__index))) {0_i32} else {external::op_add_int((var_a__index), (1_i32))};
        if (10_i32) <= (var_a__index) {break} else {()};
        var_a__index
        } /*Iter range_4: integer*/;
      { //block_5: void
        if (var_a) == (2_i32) { //block_6: void
          continue;
          } /*block_6: void*/ else {()};
        if (5_i32) < (var_a) { //block_7: void
          return var_b;
          } /*block_7: void*/ else {()};
        var_b = external::op_add_int((var_b), (var_a));
        } /*block_5: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  var_b
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    n_routine(stores)
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 13";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (13_i32), _pre13, "continue_loop", 6_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_continue_loop() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
