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

fn n_to_text(stores: &mut Stores, mut var___work_1: &mut String) -> Str { //block_1: text["res"]
  *var___work_1 = "".to_string();
  let mut var_res: String = "aa ".to_string();
  { //For block_2: void
    let mut var__i__index: i32 = i32::MIN;
    loop { //For loop_3
      let mut var__i: i32 = { //Iter range_4: integer
        var__i__index = if !(external::op_conv_bool_from_int((var__i__index))) {0_i32} else {external::op_add_int((var__i__index), (1_i32))};
        if (2_i32) <= (var__i__index) {break} else {()};
        var__i__index
        } /*Iter range_4: integer*/;
      { //block_5: void
        var_res += "b";
        } /*block_5: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  Str::new({ //Add text_6: text["__work_1"]
    var___work_1.clear();
    *var___work_1 += &var_res;
    *var___work_1 += " cc";
    ;
    var___work_1
    } /*Add text_6: text["__work_1"]*/)
  } /*block_1: text["res"]*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_3: String = "".to_string();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "1".to_string();
      let _pre13 = n_to_text(stores, &mut var___work_2);
      ops::format_text(&mut var___work_1, _pre13, 0_i32, -1, 32);
      var___work_1 += "2";
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre14 = { //Formatted string_5: text["__work_3"]
    var___work_3 = "Test failed ".to_string();
    ops::format_text(&mut var___work_3, &var_test_value, 0_i32, -1, 32);
    var___work_3 += " != \"1aa bb cc2\"";
    &var___work_3
    } /*Formatted string_5: text["__work_3"]*/;
  n_assert(stores, (&var_test_value) == ("1aa bb cc2"), _pre14, "string_fn", 12_i32);
  ;
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_string_fn() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
