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
    let s = db.structure("Elm", 0); // 18
    db.field(s, "a", 0);
    db.field(s, "b", 0);
    let s = db.structure("main_vector<Elm>", 0); // 19
    let vec_vector = db.vector(18);
    db.field(s, "vector", vec_vector);
    db.vector(18);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    OpDatabase(stores, var___ref_1, 19_i32);
    let mut var_v: DbRef = DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))};
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var_v, 20_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (1_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (2_i32));};
    OpFinishRecord(stores, var_v, var__elm_1, 20_i32, 65535_i32);
    var__elm_1 = OpNewRecord(stores, var_v, 20_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (12_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (13_i32));};
    OpFinishRecord(stores, var_v, var__elm_1, 20_i32, 65535_i32);
    var__elm_1 = OpNewRecord(stores, var_v, 20_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (4_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (5_i32));};
    OpFinishRecord(stores, var_v, var__elm_1, 20_i32, 65535_i32);
    {let db = (vector::get_vector(&(var_v), u32::from((8_i32)), (2_i32), &s.database.allocations)); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (6_i32));};
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_v, 20_i32, 0_i32);
      var___work_1 += " sizeof ";
      ops::format_long(&mut var___work_1, external::op_conv_long_from_int((8_i32)), 10 as u8, 0_i32, 32 as u8, false, false);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"[{a:1,b:2},{a:12,b:13},{a:4,b:6}] sizeof 8\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("[{a:1,b:2},{a:12,b:13},{a:4,b:6}] sizeof 8"), _pre13, "format_object", 12_i32);
  ;
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_format_object() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
