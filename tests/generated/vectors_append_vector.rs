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
    db.value(e, "C", u16::MAX);
    let s = db.structure("main_vector<Val>", 0); // 19
    let vec_vector = db.vector(18);
    db.field(s, "vector", vec_vector);
    db.vector(18);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_3: DbRef = stores.null();
  let mut var___ref_2: DbRef = stores.null();
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    OpDatabase(stores, var___ref_1, 19_i32);
    let mut var_v: DbRef = DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))};
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var_v, 20_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((1_u8)));};
    OpFinishRecord(stores, var_v, var__elm_1, 20_i32, 65535_i32);
    var__elm_1 = OpNewRecord(stores, var_v, 20_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((1_u8)));};
    OpFinishRecord(stores, var_v, var__elm_1, 20_i32, 65535_i32);
    let mut var__elm_2: DbRef = OpNewRecord(stores, var_v, 20_i32, 65535_i32);
    {let db = (var__elm_2); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((2_u8)));};
    OpFinishRecord(stores, var_v, var__elm_2, 20_i32, 65535_i32);
    let _pre13 = { //Vector_3: vector<Val>["__ref_2"]
      OpDatabase(stores, var___ref_2, 19_i32);
      let mut var__vec_3: DbRef = DbRef {store_nr: (var___ref_2).store_nr, rec: (var___ref_2).rec, pos: (var___ref_2).pos + u32::from((0_i32))};
      {let db = (var___ref_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
      let mut var__elm_4: DbRef = OpNewRecord(stores, var__vec_3, 20_i32, 65535_i32);
      {let db = (var__elm_4); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((3_u8)));};
      OpFinishRecord(stores, var__vec_3, var__elm_4, 20_i32, 65535_i32);
      var__vec_3
      } /*Vector_3: vector<Val>["__ref_2"]*/;
    s.database.vector_add(&(var_v), &(_pre13), (18_i32));;
    let _pre14 = { //Vector_4: vector<Val>["__ref_3"]
      OpDatabase(stores, var___ref_3, 19_i32);
      let mut var__vec_5: DbRef = DbRef {store_nr: (var___ref_3).store_nr, rec: (var___ref_3).rec, pos: (var___ref_3).pos + u32::from((0_i32))};
      {let db = (var___ref_3); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
      let mut var__elm_6: DbRef = OpNewRecord(stores, var__vec_5, 20_i32, 65535_i32);
      {let db = (var__elm_6); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((1_u8)));};
      OpFinishRecord(stores, var__vec_5, var__elm_6, 20_i32, 65535_i32);
      var__vec_5
      } /*Vector_4: vector<Val>["__ref_3"]*/;
    s.database.vector_add(&(var_v), &(_pre14), (18_i32));;
    { //Formatted string_5: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_v, 20_i32, 0_i32);
      &var___work_1
      } /*Formatted string_5: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre15 = { //Formatted string_6: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"[A,A,B,C,A]\"";
    &var___work_2
    } /*Formatted string_6: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("[A,A,B,C,A]"), _pre15, "append_vector", 6_i32);
  ;
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  OpFreeRef(stores, var___ref_2);
  OpFreeRef(stores, var___ref_3);
  } /*block_1: void*/

#[test]
fn code_append_vector() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
