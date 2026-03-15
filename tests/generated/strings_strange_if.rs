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

fn n_build(stores: &mut Stores, mut var_t: &mut String) -> Str { //block_1: text["t"]
  *var_t = "abcde".to_string();
  Str::new(if (3_i32) < (t_4text_len(stores, var_t)) { //block_2: text["t"]
    var_t
    } /*block_2: text["t"]*/ else { //else_3: text["t"]
    OpConvTextFromNull(stores)
    } /*else_3: text["t"]*/)
  } /*block_1: text["t"]*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text
    n_build(stores, &mut var___work_1)
    } /*block_2: text*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"abcde\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("abcde"), _pre13, "strange_if", 12_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_strange_if() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
