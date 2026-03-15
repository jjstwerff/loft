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
    db.field(s, "key", 0);
    db.field(s, "val", 0);
    let s = db.structure("Db", 0); // 19
    let index_map = db.index(18, &[("key".to_string(), true)]);
    db.field(s, "map", index_map);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    let mut var_db: DbRef = stores.null();
    OpDatabase(stores, var_db, 19_i32);
    {let db = (var_db); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var_db, 19_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (1_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (10_i32));};
    OpFinishRecord(stores, var_db, var__elm_1, 19_i32, 0_i32);
    var__elm_1 = OpNewRecord(stores, var_db, 19_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (2_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (20_i32));};
    OpFinishRecord(stores, var_db, var__elm_1, 19_i32, 0_i32);
    var__elm_1 = OpNewRecord(stores, var_db, 19_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (3_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (30_i32));};
    OpFinishRecord(stores, var_db, var__elm_1, 19_i32, 0_i32);
    { //For block_3: void
      let mut var_r__index: i32 = OpIterate(stores, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 129_i32, 16_i32, Keys([Key { type_nr: 1, position: 0 }]), 0_i32, 0_i32);
      loop { //For loop_4
        let mut var_r: DbRef = OpStep(stores, var_r__index, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 129_i32, 16_i32);
        if !((var_r).rec != 0) { //break_5: void
          break;
          } /*break_5: void*/ else {()};
        { //block_6: void
          OpRemove(stores, var_r__index, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 129_i32, 20_i32);
          } /*block_6: void*/;
        } /*For loop_4*/;
      } /*For block_3: void*/;
    let mut var_cnt: i32 = 0_i32;
    { //For block_7: void
      let mut var_r__index: i32 = OpIterate(stores, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 129_i32, 16_i32, Keys([Key { type_nr: 1, position: 0 }]), 0_i32, 0_i32);
      loop { //For loop_8
        let mut var_r: DbRef = OpStep(stores, var_r__index, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 129_i32, 16_i32);
        if !((var_r).rec != 0) { //break_9: void
          break;
          } /*break_9: void*/ else {()};
        { //block_10: void
          var_cnt = external::op_add_int((var_cnt), (1_i32));
          } /*block_10: void*/;
        } /*For loop_8*/;
      } /*For block_7: void*/;
    OpFreeRef(stores, var_db);
    var_cnt
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_11: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 0";
    &var___work_1
    } /*Formatted string_11: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (0_i32), _pre13, "index_loop_remove_small", 15_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_index_loop_remove_small() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
