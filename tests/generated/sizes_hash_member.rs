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
    let s = db.structure("S", 0); // 18
    db.field(s, "a", 0);
    db.field(s, "b", 1);
    db.field(s, "c", 0);
    let s = db.structure("Main", 0); // 19
    let hash_s = db.hash(18, &["b".to_string()]);
    db.field(s, "s", hash_s);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    external::op_add_int((16_i32), (external::op_mul_int((100_i32), (4_i32))))
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 416";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (416_i32), _pre13, "hash_member", 7_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_hash_member() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
