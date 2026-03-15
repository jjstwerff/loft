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
    db.field(s, "nr", 0);
    db.field(s, "key", 5);
    db.field(s, "value", 0);
    let s = db.structure("Db", 0); // 19
    let index_map = db.index(18, &[("nr".to_string(), true), ("key".to_string(), false)]);
    db.field(s, "map", index_map);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    let mut var_db: DbRef = stores.null();
    OpDatabase(stores, var_db, 19_i32);
    {let db = (var_db); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var_db, 19_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (101_i32));};
    {let db = (var__elm_1); let s_val = ("One").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((8_i32)), (1_i32));};
    OpFinishRecord(stores, var_db, var__elm_1, 19_i32, 0_i32);
    var__elm_1 = OpNewRecord(stores, var_db, 19_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (92_i32));};
    {let db = (var__elm_1); let s_val = ("Two").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((8_i32)), (2_i32));};
    OpFinishRecord(stores, var_db, var__elm_1, 19_i32, 0_i32);
    var__elm_1 = OpNewRecord(stores, var_db, 19_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (83_i32));};
    {let db = (var__elm_1); let s_val = ("Three").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((8_i32)), (3_i32));};
    OpFinishRecord(stores, var_db, var__elm_1, 19_i32, 0_i32);
    var__elm_1 = OpNewRecord(stores, var_db, 19_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (83_i32));};
    {let db = (var__elm_1); let s_val = ("Four").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((8_i32)), (4_i32));};
    OpFinishRecord(stores, var_db, var__elm_1, 19_i32, 0_i32);
    var__elm_1 = OpNewRecord(stores, var_db, 19_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (83_i32));};
    {let db = (var__elm_1); let s_val = ("Five").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((8_i32)), (5_i32));};
    OpFinishRecord(stores, var_db, var__elm_1, 19_i32, 0_i32);
    var__elm_1 = OpNewRecord(stores, var_db, 19_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (63_i32));};
    {let db = (var__elm_1); let s_val = ("Six").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((8_i32)), (6_i32));};
    OpFinishRecord(stores, var_db, var__elm_1, 19_i32, 0_i32);
    let _pre14 = OpGetRecord(stores, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 20_i32, 2_i32, 101_i32, "One");
    let _pre13 = ({let db = (_pre14); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((8_i32)))} }) == (1_i32);
    n_assert(stores, _pre13, "Missing element", "index_iterator", 13_i32);
    let mut var_sum: i32 = 0_i32;
    { //For block_3: void
      let mut var__iter_2: i64 = OpIterate(stores, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 1_i32, 20_i32, Keys([Key { type_nr: 1, position: 0 }, Key { type_nr: -6, position: 4 }]), 1_i32, 83_i32, 2_i32, 92_i32, "Two");
      loop { //For loop_4
        let mut var_v: DbRef = { //Iterate keys_5: index<Elm,[("nr", true), ("key", false)]>["db"]
          OpStep(stores, var__iter_2, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 1_i32, 20_i32)
          } /*Iterate keys_5: index<Elm,[("nr", true), ("key", false)]>["db"]*/;
        if !((var_v).rec != 0) { //break_6: void
          break;
          } /*break_6: void*/ else {()};
        { //block_7: void
          var_sum = external::op_add_int((external::op_mul_int((var_sum), (10_i32))), ({let db = (var_v); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((8_i32)))} }));
          } /*block_7: void*/;
        } /*For loop_4*/;
      } /*For block_3: void*/;
    let mut var_total: i32 = 0_i32;
    { //For block_8: void
      let mut var_r__index: i32 = OpIterate(stores, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 129_i32, 20_i32, Keys([Key { type_nr: 1, position: 0 }, Key { type_nr: -6, position: 4 }]), 0_i32, 0_i32);
      loop { //For loop_9
        let mut var_r: DbRef = OpStep(stores, var_r__index, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 129_i32, 20_i32);
        if !((var_r).rec != 0) { //break_10: void
          break;
          } /*break_10: void*/ else {()};
        { //block_11: void
          var_total = external::op_add_int((var_total), ({let db = (var_r); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((8_i32)))} }));
          } /*block_11: void*/;
        } /*For loop_9*/;
      } /*For block_8: void*/;
    let _pre15 = { //Formatted string_12: text["__work_1"]
      var___work_1 = "Incorrect total ".to_string();
      ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_total)), 10 as u8, 0_i32, 32 as u8, false, false);
      &var___work_1
      } /*Formatted string_12: text["__work_1"]*/;
    n_assert(stores, (var_total) == (21_i32), _pre15, "index_iterator", 22_i32);
    let _pre16 = !((OpGetRecord(stores, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 20_i32, 2_i32, 12_i32, "")).rec != 0);
    n_assert(stores, _pre16, "No element", "index_iterator", 23_i32);
    let _pre17 = !((OpGetRecord(stores, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 20_i32, 2_i32, 83_i32, "One")).rec != 0);
    n_assert(stores, _pre17, "No element", "index_iterator", 24_i32);
    OpFreeRef(stores, var_db);
    var_sum
    } /*block_2: integer*/;
  let _pre18 = { //Formatted string_13: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_long(&mut var___work_2, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_2 += " != 345";
    &var___work_2
    } /*Formatted string_13: text["__work_2"]*/;
  n_assert(stores, (var_test_value) == (345_i32), _pre18, "index_iterator", 27_i32);
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_index_iterator() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
