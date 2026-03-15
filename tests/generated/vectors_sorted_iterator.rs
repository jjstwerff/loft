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
    let sorted_map = db.sorted(18, &[("nr".to_string(), false), ("key".to_string(), true)]);
    db.field(s, "map", sorted_map);
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
    var__elm_1 = OpNewRecord(stores, var_db, 19_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (61_i32));};
    {let db = (var__elm_1); let s_val = ("Seven").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((8_i32)), (7_i32));};
    OpFinishRecord(stores, var_db, var__elm_1, 19_i32, 0_i32);
    let mut var_sum: i32 = 0_i32;
    let _pre14 = OpGetRecord(stores, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 20_i32, 2_i32, 83_i32, "Five");
    let _pre13 = ({let db = (_pre14); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((8_i32)))} }) == (5_i32);
    let _pre15 = { //Formatted string_3: text["__work_1"]
      var___work_1 = "Incorrect element ".to_string();
      let _pre17 = OpGetRecord(stores, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 20_i32, 2_i32, 83_i32, "Five");
      let _pre16 = external::op_conv_long_from_int(({let db = (_pre17); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((8_i32)))} }));
      ops::format_long(&mut var___work_1, _pre16, 10 as u8, 0_i32, 32 as u8, false, false);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/;
    n_assert(stores, _pre13, { //Formatted string_3: text["__work_1"]
      var___work_1 = "Incorrect element ".to_string();
      let _pre19 = _pre14;
      let _pre18 = external::op_conv_long_from_int(({let db = (_pre19); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((8_i32)))} }));
      ops::format_long(&mut var___work_1, _pre18, 10 as u8, 0_i32, 32 as u8, false, false);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/, "sorted_iterator", 15_i32);
    { //For block_4: void
      let mut var__iter_2: i64 = OpIterate(stores, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 130_i32, 12_i32, Keys([Key { type_nr: -1, position: 0 }, Key { type_nr: 6, position: 4 }]), 1_i32, 84_i32, 2_i32, 63_i32, "Six");
      loop { //For loop_5
        let mut var_v: DbRef = { //Iterate keys_6: sorted<Elm,[("nr", false), ("key", true)]>["db"]
          OpStep(stores, var__iter_2, DbRef {store_nr: (var_db).store_nr, rec: (var_db).rec, pos: (var_db).pos + u32::from((0_i32))}, 130_i32, 12_i32)
          } /*Iterate keys_6: sorted<Elm,[("nr", false), ("key", true)]>["db"]*/;
        if !((var_v).rec != 0) { //break_7: void
          break;
          } /*break_7: void*/ else {()};
        { //block_8: void
          var_sum = external::op_add_int((external::op_mul_int((var_sum), (10_i32))), ({let db = (var_v); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((8_i32)))} }));
          } /*block_8: void*/;
        } /*For loop_5*/;
      } /*For block_4: void*/;
    OpFreeRef(stores, var_db);
    var_sum
    } /*block_2: integer*/;
  let _pre20 = { //Formatted string_9: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_long(&mut var___work_2, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_2 += " != 5436";
    &var___work_2
    } /*Formatted string_9: text["__work_2"]*/;
  n_assert(stores, (var_test_value) == (5436_i32), _pre20, "sorted_iterator", 21_i32);
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_sorted_iterator() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
