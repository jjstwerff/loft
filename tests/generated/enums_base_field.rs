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
    let e = db.enumerate("Val");
    db.value(e, "A", u16::MAX);
    db.value(e, "B", u16::MAX);
    let s = db.structure("A", 1); // 19
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "n", 0);
    let s = db.structure("B", 2); // 20
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "n", 0);
    db.finish();
}

fn n_get_n(stores: &mut Stores, mut var_v: DbRef) -> i32 { //block_1: integer
  {let db = (var_v); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} }
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    let _pre13 = { //Object_3: ref(A)["__ref_1"]
      OpDatabase(stores, var___ref_1, 19_i32);
      {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (42_i32));};
      {let db = (var___ref_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((1_u8)));};
      var___ref_1
      } /*Object_3: ref(A)["__ref_1"]*/;
    n_get_n(stores, _pre13)
    } /*block_2: integer*/;
  let _pre14 = { //Formatted string_4: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 42";
    &var___work_1
    } /*Formatted string_4: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (42_i32), _pre14, "base_field", 10_i32);
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_base_field() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
