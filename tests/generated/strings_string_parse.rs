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

fn n_parse(stores: &mut Stores, mut var_s: Str) -> i32 { //block_1: integer
  { //For block_2: void
    let mut var_t__index: i32 = i32::MIN;
    loop { //For loop_3
      let mut var_t: i32 = { //Iter range_4: integer
        var_t__index = if !(external::op_conv_bool_from_int((var_t__index))) {0_i32} else {external::op_add_int((var_t__index), (1_i32))};
        if (300_i32) <= (var_t__index) {break} else {()};
        var_t__index
        } /*Iter range_4: integer*/;
      { //block_5: void
        let mut var_l: i32 = external::text_character((&var_s), (var_t));
        if if !((var_l).is_alphanumeric()) {(if (var_l) == char::from(0) { i32::MIN } else { (var_l) as i32 }) != (if (char::from_u32(95_u32).unwrap_or('\0')) == char::from(0) { i32::MIN } else { (char::from_u32(95_u32).unwrap_or('\0')) as i32 })} else {false} { //block_6: void
          return var_t;
          } /*block_6: void*/ else {()};
        } /*block_5: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  return 0_i32
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    n_parse(stores, "if_cond ")
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 7";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (7_i32), _pre13, "string_parse", 15_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_string_parse() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
