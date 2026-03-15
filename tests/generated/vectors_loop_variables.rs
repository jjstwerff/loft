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
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      var___work_1 += "[";
      let mut var_x__index: i32 = i32::MIN;
      let mut var_x__count: i32 = 0_i32;
      loop { //Append Iter_4
        let mut var__val_1: i32 = { //Iter For_5: integer
          let mut var_x: i32 = { //Iter range_6: integer
            var_x__index = if !(external::op_conv_bool_from_int((var_x__index))) {0_i32} else {external::op_add_int((var_x__index), (1_i32))};
            if (10_i32) <= (var_x__index) {break} else {()};
            var_x__index
            } /*Iter range_6: integer*/;
          if if (var_x) != (0_i32) {(external::op_rem_int((var_x), (3_i32))) == (0_i32)} else {false} {()} else {continue};
          { //block_7: integer
            if (var_x__count) == (0_i32) { //block_8: integer
              var_x
              } /*block_8: integer*/ else { //block_9: integer
              external::op_mul_int((var_x), (2_i32))
              } /*block_9: integer*/
            } /*block_7: integer*/
          } /*Iter For_5: integer*/;
        if (0_i32) < (var_x__count) {var___work_1 += ","} else {()};
        var_x__count = external::op_add_int((var_x__count), (1_i32));
        ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var__val_1)), 10 as u8, 0_i32, 32 as u8, false, false);
        } /*Append Iter_4*/;
      var___work_1 += "]";
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_10: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"[3,12,18]\"";
    &var___work_2
    } /*Formatted string_10: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("[3,12,18]"), _pre13, "loop_variables", 4_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_loop_variables() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
