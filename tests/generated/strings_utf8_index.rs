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
    let mut var_a: String = "♥😃".to_string();
    { //Add text_3: text["__work_1"]
      OpClearText(stores, &var___work_1);
      {let c = external::text_character((&var_a), (0_i32)); if c != 0 { var___work_1.push(ops::to_char(c)); } };
      {let c = external::text_character((&var_a), (1_i32)); if c != 0 { var___work_1.push(ops::to_char(c)); } };
      {let c = external::text_character((&var_a), (2_i32)); if c != 0 { var___work_1.push(ops::to_char(c)); } };
      {let c = external::text_character((&var_a), (3_i32)); if c != 0 { var___work_1.push(ops::to_char(c)); } };
      {let c = external::text_character((&var_a), (4_i32)); if c != 0 { var___work_1.push(ops::to_char(c)); } };
      {let c = external::text_character((&var_a), (5_i32)); if c != 0 { var___work_1.push(ops::to_char(c)); } };
      {let c = external::text_character((&var_a), (6_i32)); if c != 0 { var___work_1.push(ops::to_char(c)); } };
      var___work_1 += ".";
      {let c = external::text_character((&var_a), (7_i32)); if c != 0 { var___work_1.push(ops::to_char(c)); } };
      ;
      &var___work_1
      } /*Add text_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"♥♥♥😃😃😃😃.\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("♥♥♥😃😃😃😃."), _pre13, "utf8_index", 4_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_utf8_index() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
