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
    let mut var_a: i32 = 0_i32;
    { //For block_3: void
      let mut var__t__index: i32 = i32::MIN;
      loop { //For loop_4
        let mut var__t: i32 = { //Iter range_5: integer
          var__t__index = if !(external::op_conv_bool_from_int((var__t__index))) {1_i32} else {external::op_add_int((var__t__index), (1_i32))};
          if (5_i32) <= (var__t__index) {break} else {()};
          var__t__index
          } /*Iter range_5: integer*/;
        { //block_6: void
          let mut var_b: String = "123".to_string();
          var_a = external::op_add_int((var_a), ((&var_b).parse().unwrap_or(i32::MIN)));
          if (200_i32) < (var_a) { //block_7: void
            ;
            break;
            } /*block_7: void*/ else {()};
          ;
          } /*block_6: void*/;
        } /*For loop_4*/;
      } /*For block_3: void*/;
    var_a
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_8: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 246";
    &var___work_1
    } /*Formatted string_8: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (246_i32), _pre13, "loop_variable", 4_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_loop_variable() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
