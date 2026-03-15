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
    db.value(e, "Small", u16::MAX);
    db.value(e, "Large", u16::MAX);
    let s = db.structure("Small", 1); // 19
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let byte_n = db.byte(0, true);
    db.field(s, "n", byte_n);
    let s = db.structure("Large", 2); // 21
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "n", 1);
    db.finish();
}

fn n_get_size(stores: &mut Stores, mut var_v: DbRef) -> i32 { //block_1: integer
  OpSizeofRef(stores, var_v)
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_2: DbRef = stores.null();
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    if (n_get_size(stores, { //Object_3: ref(Small)["__ref_1"]
      OpDatabase(stores, var___ref_1, 19_i32);
      {let db = (var___ref_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((7_i32)), i32::from((0_i32)), (1_i32));};
      {let db = (var___ref_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((1_u8)));};
      var___ref_1
      } /*Object_3: ref(Small)["__ref_1"]*/)) == (n_get_size(stores, { //Object_4: ref(Large)["__ref_2"]
      OpDatabase(stores, var___ref_2, 21_i32);
      {let db = (var___ref_2); stores.store_mut(&db).set_long(db.rec, db.pos + u32::from((8_i32)), (42_i64));};
      {let db = (var___ref_2); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((2_u8)));};
      var___ref_2
      } /*Object_4: ref(Large)["__ref_2"]*/)) { //block_5: integer
      1_i32
      } /*block_5: integer*/ else { //block_6: integer
      0_i32
      } /*block_6: integer*/
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_7: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 0";
    &var___work_1
    } /*Formatted string_7: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (0_i32), _pre13, "sizeof_enum_structs", 10_i32);
  ;
  OpFreeRef(stores, var___ref_1);
  OpFreeRef(stores, var___ref_2);
  } /*block_1: void*/

#[test]
fn code_sizeof_enum_structs() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
