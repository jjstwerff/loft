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

fn n_choice(stores: &mut Stores, mut var_a: Str, mut var_b: Str) -> Str { //block_1: text["a", "b"]
  Str::new(if (t_4text_len(stores, &var_b)) < (t_4text_len(stores, &var_a)) { //block_2: text["a"]
    &var_a
    } /*block_2: text["a"]*/ else { //block_3: text["a"]
    &var_b
    } /*block_3: text["a"]*/)
  } /*block_1: text["a", "b"]*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_4: String = "".to_string();
  let mut var___work_3: String = "".to_string();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_3"]
    { //Add text_3: text["__work_3"]
      OpClearText(stores, &var___work_3);
      let _pre14 = { //Formatted string_4: text["__work_1"]
        var___work_1 = "".to_string();
        ops::format_long(&mut var___work_1, external::op_conv_long_from_int((1_i32)), 10 as u8, 3_i32, 48 as u8, false, false);
        &var___work_1
        } /*Formatted string_4: text["__work_1"]*/;
      let _pre15 = { //Formatted string_5: text["__work_2"]
        var___work_2 = "".to_string();
        ops::format_long(&mut var___work_2, external::op_conv_long_from_int((2_i32)), 10 as u8, 0_i32, 32 as u8, false, false);
        var___work_2 += "1";
        &var___work_2
        } /*Formatted string_5: text["__work_2"]*/;
      let _pre13 = n_choice(stores, _pre14, _pre15);
      var___work_3 += _pre13;
      let _pre16 = n_choice(stores, "2", "");
      var___work_3 += _pre16;
      &var___work_3
      } /*Add text_3: text["__work_3"]*/
    } /*block_2: text["__work_3"]*/.to_string();
  let _pre17 = { //Formatted string_6: text["__work_4"]
    var___work_4 = "Test failed ".to_string();
    ops::format_text(&mut var___work_4, &var_test_value, 0_i32, -1, 32);
    var___work_4 += " != \"0012\"";
    &var___work_4
    } /*Formatted string_6: text["__work_4"]*/;
  n_assert(stores, (&var_test_value) == ("0012"), _pre17, "call", 6_i32);
  ;
  ;
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_call() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
