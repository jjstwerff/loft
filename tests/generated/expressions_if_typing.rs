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
  let mut var_test_value = { //block_2: text
    let mut var_a: String = "12".to_string();
    let _ret = if (t_4text_len(stores, &var_a)) == (2_i32) { //block_3: text
      OpConvTextFromNull(stores)
      } /*block_3: text*/ else { //block_4: text
      "error"
      } /*block_4: text*/;
    ;
    _ret
    } /*block_2: text*/.to_string();
  let _pre13 = (&var_test_value) == (OpConvTextFromNull(stores));
  let _pre14 = { //Formatted string_5: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_text(&mut var___work_1, &var_test_value, 0_i32, -1, 32);
    var___work_1 += " != null";
    &var___work_1
    } /*Formatted string_5: text["__work_1"]*/;
  n_assert(stores, _pre13, _pre14, "if_typing", 4_i32);
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_if_typing() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
