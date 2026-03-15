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

fn n_add(stores: &mut Stores, mut var_a: &mut String, mut var_b: Str) { //block_1: void
  *var_a += &var_b;
  } /*block_1: void*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_v: String = "".to_string();
  let mut var_test_value = { //block_2: text["v"]
    var_v = "Hello".to_string();
    let _pre13 = ;
    _pre13n_pre13__pre13a_pre13d_pre13d_pre13(_pre13s_pre13t_pre13o_pre13r_pre13e_pre13s_pre13,_pre13 _pre13,_pre13 _pre13"_pre13 _pre13w_pre13o_pre13r_pre13l_pre13d_pre13!_pre13"_pre13)_pre13;
    &var_v
    } /*block_2: text["v"]*/.to_string();
  let _pre14 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_text(&mut var___work_1, &var_test_value, 0_i32, -1, 32);
    var___work_1 += " != \"Hello world!\"";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (&var_test_value) == ("Hello world!"), _pre14, "reference", 8_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_reference() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
