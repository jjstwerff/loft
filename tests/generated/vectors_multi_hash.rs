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
    let e = db.enumerate("Cat");
    db.value(e, "A", u16::MAX);
    db.value(e, "B", u16::MAX);
    db.value(e, "C", u16::MAX);
    let s = db.structure("Count", 0); // 19
    db.field(s, "c", 18);
    db.field(s, "t", 5);
    db.field(s, "v", 0);
    let s = db.structure("Counting", 0); // 20
    let sorted_v = db.sorted(19, &[("t".to_string(), true), ("v".to_string(), true)]);
    db.field(s, "v", sorted_v);
    let hash_h = db.hash(19, &["c".to_string(), "t".to_string()]);
    db.field(s, "h", hash_h);
    db.finish();
}

fn n_fill(stores: &mut Stores, mut var_c: DbRef) { //block_1: void
  let mut var__elm_1: DbRef = OpNewRecord(stores, var_c, 20_i32, 0_i32);
  {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((8_i32)), 0, i32::from((1_u8)));};
  {let db = (var__elm_1); let s_val = ("One").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (1_i32));};
  OpFinishRecord(stores, var_c, var__elm_1, 20_i32, 0_i32);
  var__elm_1 = OpNewRecord(stores, var_c, 20_i32, 0_i32);
  {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((8_i32)), 0, i32::from((2_u8)));};
  {let db = (var__elm_1); let s_val = ("Two").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (2_i32));};
  OpFinishRecord(stores, var_c, var__elm_1, 20_i32, 0_i32);
  var__elm_1 = OpNewRecord(stores, var_c, 20_i32, 0_i32);
  {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((8_i32)), 0, i32::from((3_u8)));};
  {let db = (var__elm_1); let s_val = ("Two").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (20_i32));};
  OpFinishRecord(stores, var_c, var__elm_1, 20_i32, 0_i32);
  var__elm_1 = OpNewRecord(stores, var_c, 20_i32, 0_i32);
  {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((8_i32)), 0, i32::from((1_u8)));};
  {let db = (var__elm_1); let s_val = ("Three").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (3_i32));};
  OpFinishRecord(stores, var_c, var__elm_1, 20_i32, 0_i32);
  var__elm_1 = OpNewRecord(stores, var_c, 20_i32, 0_i32);
  {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((8_i32)), 0, i32::from((3_u8)));};
  {let db = (var__elm_1); let s_val = ("Four").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (4_i32));};
  OpFinishRecord(stores, var_c, var__elm_1, 20_i32, 0_i32);
  } /*block_1: void*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    let mut var_c: DbRef = stores.null();
    OpDatabase(stores, var_c, 20_i32);
    {let db = (var_c); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    {let db = (var_c); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (0_i32));};
    n_fill(stores, var_c);
    let _pre15 = OpGetRecord(stores, DbRef {store_nr: (var_c).store_nr, rec: (var_c).rec, pos: (var_c).pos + u32::from((4_i32))}, 22_i32, 2_i32, 1_u8, "Three");
    let _pre14 = {let db = (_pre15); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} };
    let _pre17 = OpGetRecord(stores, DbRef {store_nr: (var_c).store_nr, rec: (var_c).rec, pos: (var_c).pos + u32::from((4_i32))}, 22_i32, 2_i32, 3_u8, "Two");
    let _pre16 = {let db = (_pre17); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} };
    let _pre13 = external::op_add_int((_pre14), (_pre16));
    let _pre19 = OpGetRecord(stores, DbRef {store_nr: (var_c).store_nr, rec: (var_c).rec, pos: (var_c).pos + u32::from((0_i32))}, 21_i32, 2_i32, "Four", 4_i32);
    let _pre18 = {let db = (_pre19); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} };
    let _ret = external::op_add_int((_pre13), (_pre18));
    OpFreeRef(stores, var_c);
    _ret
    } /*block_2: integer*/;
  let _pre20 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 27";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (27_i32), _pre20, "multi_hash", 17_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_multi_hash() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
