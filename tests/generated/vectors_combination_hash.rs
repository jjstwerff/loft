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
    let s = db.structure("Count", 0); // 18
    db.field(s, "t", 5);
    db.field(s, "v", 0);
    let s = db.structure("Counting", 0); // 19
    let vec_v = db.vector(18);
    db.field(s, "v", vec_v);
    let hash_h = db.hash(18, &["t".to_string()]);
    db.field(s, "h", hash_h);
    db.vector(18);
    db.finish();
}

fn n_fill(stores: &mut Stores, mut var_c: DbRef) { //block_1: void
  let mut var__elm_1: DbRef = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_1); let s_val = ("One").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (1_i32));};
  OpFinishRecord(stores, var_c, var__elm_1, 19_i32, 0_i32);
  var__elm_1 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_1); let s_val = ("Two").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (2_i32));};
  OpFinishRecord(stores, var_c, var__elm_1, 19_i32, 0_i32);
  let mut var__elm_2: DbRef = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Three").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (3_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  var__elm_2 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Four").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (4_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  var__elm_2 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Five").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (5_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  var__elm_2 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Six").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (6_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  var__elm_2 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Seven").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (7_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  var__elm_2 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Eight").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (8_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  var__elm_2 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Nine").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (9_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  var__elm_2 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Ten").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (10_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  var__elm_2 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Eleven").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (11_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  var__elm_2 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Twelve").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (12_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  var__elm_2 = OpNewRecord(stores, var_c, 19_i32, 0_i32);
  {let db = (var__elm_2); let s_val = ("Thirteen").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (13_i32));};
  OpFinishRecord(stores, var_c, var__elm_2, 19_i32, 0_i32);
  } /*block_1: void*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    let mut var_c: DbRef = stores.null();
    OpDatabase(stores, var_c, 19_i32);
    {let db = (var_c); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    {let db = (var_c); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (0_i32));};
    n_fill(stores, var_c);
    let _pre13 = !((OpGetRecord(stores, DbRef {store_nr: (var_c).store_nr, rec: (var_c).rec, pos: (var_c).pos + u32::from((4_i32))}, 21_i32, 1_i32, "None")).rec != 0);
    n_assert(stores, _pre13, "No element", "combination_hash", 26_i32);
    let mut var_add: i32 = 0_i32;
    { //For block_3: void
      let mut var__vector_1: DbRef = DbRef {store_nr: (var_c).store_nr, rec: (var_c).rec, pos: (var_c).pos + u32::from((0_i32))};
      let mut var_v__index: i32 = -1_i32;
      loop { //For loop_4
        let mut var_v: DbRef = { //iter next_5: ref(Count)
          var_v__index = external::op_add_int((var_v__index), (1_i32));
          stores.get_ref(&vector::get_vector(&(var__vector_1), 4, (var_v__index), &s.database.allocations), 0)
          } /*iter next_5: ref(Count)*/;
        if !((var_v).rec != 0) { //break_6: void
          break;
          } /*break_6: void*/ else {()};
        { //block_7: void
          var_add = external::op_add_int((var_add), ({let db = (var_v); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} }));
          } /*block_7: void*/;
        } /*For loop_4*/;
      } /*For block_3: void*/;
    n_assert(stores, (var_add) == (91_i32), "Incorrect sum", "combination_hash", 31_i32);
    let _pre15 = OpGetRecord(stores, DbRef {store_nr: (var_c).store_nr, rec: (var_c).rec, pos: (var_c).pos + u32::from((4_i32))}, 21_i32, 1_i32, "Five");
    let _pre14 = {let db = (_pre15); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} };
    let _pre17 = OpGetRecord(stores, DbRef {store_nr: (var_c).store_nr, rec: (var_c).rec, pos: (var_c).pos + u32::from((4_i32))}, 21_i32, 1_i32, "Seven");
    let _pre16 = {let db = (_pre17); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} };
    let _ret = external::op_add_int((_pre14), (_pre16));
    OpFreeRef(stores, var_c);
    _ret
    } /*block_2: integer*/;
  let _pre18 = { //Formatted string_8: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 12";
    &var___work_1
    } /*Formatted string_8: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (12_i32), _pre18, "combination_hash", 34_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_combination_hash() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
