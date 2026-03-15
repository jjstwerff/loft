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
    let s = db.structure("T", 0); // 18
    db.field(s, "n", 5);
    let short_v = db.short(0, true);
    db.field(s, "v", short_v);
    let s = db.structure("N", 0); // 20
    let vec_d = db.vector(18);
    db.field(s, "d", vec_d);
    let hash_h = db.hash(18, &["n".to_string()]);
    db.field(s, "h", hash_h);
    db.vector(18);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    let mut var_s: DbRef = stores.null();
    OpDatabase(stores, var_s, 20_i32);
    {let db = (var_s); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var_s, 20_i32, 0_i32);
    {let db = (var__elm_1); let s_val = ("a").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    {let db = (var__elm_1); stores.store_mut(&db).set_short(db.rec, db.pos + u32::from((4_i32)), i32::from((0_i32)), (12_i32));};
    OpFinishRecord(stores, var_s, var__elm_1, 20_i32, 0_i32);
    {let db = (var_s); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (0_i32));};
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, stores.get_ref(&(vector::get_vector(&(DbRef {store_nr: (var_s).store_nr, rec: (var_s).rec, pos: (var_s).pos + u32::from((0_i32))}), u32::from((6_i32)), (0_i32), &s.database.allocations)), u32::from((0_i32))), 18_i32, 0_i32);
      var___work_1 += " v=";
      ops::format_long(&mut var___work_1, external::op_conv_long_from_int(({let db = (stores.get_ref(&(vector::get_vector(&(DbRef {store_nr: (var_s).store_nr, rec: (var_s).rec, pos: (var_s).pos + u32::from((0_i32))}), u32::from((6_i32)), (0_i32), &s.database.allocations)), u32::from((0_i32)))); stores.store(&db).get_short(db.rec, db.pos + u32::from((4_i32)), i32::from((0_i32)))})), 10 as u8, 0_i32, 32 as u8, false, false);
      OpFreeRef(stores, var_s);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"{n:\"a\",v:12} v=12\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("{n:\"a\",v:12} v=12"), _pre13, "get_object_value", 7_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_get_object_value() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
