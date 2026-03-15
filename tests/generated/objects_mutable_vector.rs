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
    db.vector(0);
    let s = db.structure("main_vector<integer>", 0); // 19
    let vec_vector = db.vector(0);
    db.field(s, "vector", vec_vector);
    db.finish();
}

fn n_add(stores: &mut Stores, mut var_r: &mut DbRef, mut var_val: i32) { //block_1: void
  let mut var__elm_1: DbRef = OpNewRecord(stores, var_r, 18_i32, 65535_i32);
  {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (var_val));};
  OpFinishRecord(stores, var_r, var__elm_1, 18_i32, 65535_i32);
  } /*block_1: void*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    OpDatabase(stores, var___ref_1, 19_i32);
    let mut var_v: DbRef = DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))};
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var_v, 18_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (1_i32));};
    OpFinishRecord(stores, var_v, var__elm_1, 18_i32, 65535_i32);
    let _pre13 = ;
    _pre13n_pre13__pre13a_pre13d_pre13d_pre13(_pre13s_pre13t_pre13o_pre13r_pre13e_pre13s_pre13,_pre13 _pre13,_pre13 _pre132_pre13__pre13i_pre133_pre132_pre13)_pre13;
    let _pre14 = ;
    _pre14n_pre14__pre14a_pre14d_pre14d_pre14(_pre14s_pre14t_pre14o_pre14r_pre14e_pre14s_pre14,_pre14 _pre14,_pre14 _pre143_pre14__pre14i_pre143_pre142_pre14)_pre14;
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_v, 18_i32, 0_i32);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre15 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"[1,2,3]\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("[1,2,3]"), _pre15, "mutable_vector", 8_i32);
  ;
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_mutable_vector() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
