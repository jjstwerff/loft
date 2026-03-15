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
    let e = db.enumerate("Value");
    db.value(e, "S", u16::MAX);
    db.value(e, "I", u16::MAX);
    db.value(e, "H", u16::MAX);
    let s = db.structure("S", 1); // 19
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let sorted_data = db.sorted(20, &[("nr".to_string(), true)]);
    db.field(s, "data", sorted_data);
    let s = db.structure("Sort", 0); // 20
    db.field(s, "nr", 0);
    db.field(s, "d", 18);
    let s = db.structure("I", 2); // 22
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let index_data = db.index(23, &[("nr".to_string(), true)]);
    db.field(s, "data", index_data);
    let s = db.structure("Ind", 0); // 23
    db.field(s, "nr", 0);
    db.field(s, "d", 18);
    let s = db.structure("H", 3); // 25
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let hash_data = db.hash(26, &["name".to_string()]);
    db.field(s, "data", hash_data);
    let s = db.structure("Elm", 0); // 26
    db.field(s, "name", 5);
    db.field(s, "d", 18);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    1_i32
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 1";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (1_i32), _pre13, "types", 26_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_types() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
