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
  let mut var___work_3: String = "".to_string();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_2"]
    let mut var_a: i64 = 1_i64;
    let mut var_b: String = "".to_string();
    { //For block_3: void
      let mut var_n__index: i32 = i32::MIN;
      loop { //For loop_4
        let mut var_n: i32 = { //Iter range_5: integer
          var_n__index = if !(external::op_conv_bool_from_int((var_n__index))) {1_i32} else {external::op_add_int((var_n__index), (1_i32))};
          if (4_i32) <= (var_n__index) {break} else {()};
          var_n__index
          } /*Iter range_5: integer*/;
        { //block_6: void
          let mut var_t: String = "1".to_string();
          var_b += "n";
          let _pre13 = { //Formatted string_7: text["__work_1"]
            var___work_1 = ":".to_string();
            ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_n)), 10 as u8, 0_i32, 32 as u8, false, false);
            &var___work_1
            } /*Formatted string_7: text["__work_1"]*/;
          var_b += _pre13;
          var_b += "=";
          { //For block_8: void
            let mut var__m__index: i32 = i32::MIN;
            loop { //For loop_9
              let mut var__m: i32 = { //Iter range_10: integer
                var__m__index = if !(external::op_conv_bool_from_int((var__m__index))) {1_i32} else {external::op_add_int((var__m__index), (1_i32))};
                if (var_n) <= (var__m__index) {break} else {()};
                var__m__index
                } /*Iter range_10: integer*/;
              { //block_11: void
                var_t += "2";
                } /*block_11: void*/;
              } /*For loop_9*/;
            } /*For block_8: void*/;
          var_b += &var_t;
          var_b += " ";
          var_a = external::op_add_long((var_a), ((&var_t).parse().unwrap_or(i64::MIN)));
          ;
          } /*block_6: void*/;
        } /*For loop_4*/;
      } /*For block_3: void*/;
    { //Formatted string_12: text["__work_2"]
      var___work_2 = "".to_string();
      ops::format_long(&mut var___work_2, var_a, 10 as u8, 0_i32, 32 as u8, false, false);
      var___work_2 += " via ";
      ops::format_text(&mut var___work_2, &var_b, 0_i32, -1, 32);
      ;
      &var___work_2
      } /*Formatted string_12: text["__work_2"]*/
    } /*block_2: text["__work_2"]*/.to_string();
  let _pre14 = { //Formatted string_13: text["__work_3"]
    var___work_3 = "Test failed ".to_string();
    ops::format_text(&mut var___work_3, &var_test_value, 0_i32, -1, 32);
    var___work_3 += " != \"136 via n:1=1 n:2=12 n:3=122 \"";
    &var___work_3
    } /*Formatted string_13: text["__work_3"]*/;
  n_assert(stores, (&var_test_value) == ("136 via n:1=1 n:2=12 n:3=122 "), _pre14, "string_scope", 17_i32);
  ;
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_string_scope() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
