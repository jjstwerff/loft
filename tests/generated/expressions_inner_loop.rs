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
  let mut var_s: String = "".to_string();
  let mut var_test_value = { //block_2: text["s"]
    var_s = "".to_string();
    { //For block_3: void
      let mut var_i__index: i32 = i32::MIN;
      loop { //For loop_4
        let mut var_i: i32 = { //Iter range_5: integer
          var_i__index = if !(external::op_conv_bool_from_int((var_i__index))) {0_i32} else {external::op_add_int((var_i__index), (1_i32))};
          if (10_i32) <= (var_i__index) {break} else {()};
          var_i__index
          } /*Iter range_5: integer*/;
        { //block_6: void
          { //For block_7: void
            let mut var_j__index: i32 = i32::MIN;
            loop { //For loop_8
              let mut var_j: i32 = { //Iter range_9: integer
                var_j__index = if !(external::op_conv_bool_from_int((var_j__index))) {0_i32} else {external::op_add_int((var_j__index), (1_i32))};
                if (10_i32) <= (var_j__index) {break} else {()};
                var_j__index
                } /*Iter range_9: integer*/;
              { //block_10: void
                if (var_i) < (var_j) { //block_11: void
                  continue;
                  } /*block_11: void*/ else {()};
                let _pre13 = { //Formatted string_12: text["__work_1"]
                  var___work_1 = "".to_string();
                  ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_i)), 10 as u8, 0_i32, 32 as u8, false, false);
                  ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_j)), 10 as u8, 0_i32, 32 as u8, false, false);
                  var___work_1 += ",";
                  &var___work_1
                  } /*Formatted string_12: text["__work_1"]*/;
                var_s += _pre13;
                if (100_i32) < (t_4text_len(stores, &var_s)) { //block_13: void
                  break;
                  } /*block_13: void*/ else {()};
                } /*block_10: void*/;
              } /*For loop_8*/;
            } /*For block_7: void*/;
          } /*block_6: void*/;
        } /*For loop_4*/;
      } /*For block_3: void*/;
    &var_s
    } /*block_2: text["s"]*/.to_string();
  let _pre14 = { //Formatted string_14: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"00,10,11,20,21,22,30,31,32,33,40,41,42,43,44,50,51,52,53,54,55,60,61,62,63,64,65,66,70,71,72,73,74,75,\"";
    &var___work_2
    } /*Formatted string_14: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("00,10,11,20,21,22,30,31,32,33,40,41,42,43,44,50,51,52,53,54,55,60,61,62,63,64,65,66,70,71,72,73,74,75,"), _pre14, "inner_loop", 18_i32);
  ;
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_inner_loop() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
