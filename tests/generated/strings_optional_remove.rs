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

fn n_last(stores: &mut Stores, mut var_filename: Str) -> Str { //block_1: text["filename"]
  let mut var_v: i32 = if let Some(v) = (&var_filename).rfind(("/")) { v as i32 } else { i32::MIN };
  Str::new(if external::op_conv_bool_from_int((var_v)) { //block_2: text["filename"]
    OpGetTextSub(stores, &var_filename, external::op_add_int((var_v), (1_i32)), 2147483647_i32)
    } /*block_2: text["filename"]*/ else { //block_3: text["filename"]
    &var_filename
    } /*block_3: text["filename"]*/)
  } /*block_1: text["filename"]*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    { //Add text_3: text["__work_1"]
      OpClearText(stores, &var___work_1);
      let _pre13 = n_last(stores, "a/b/c");
      var___work_1 += _pre13;
      let _pre14 = n_last(stores, "d");
      var___work_1 += _pre14;
      &var___work_1
      } /*Add text_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre15 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"cd\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("cd"), _pre15, "optional_remove", 13_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_optional_remove() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
