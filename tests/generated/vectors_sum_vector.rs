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

fn n_sum(stores: &mut Stores, mut var_v: DbRef) -> i32 { //block_1: integer
  let mut var_t: i32 = 0_i32;
  { //For block_2: void
    let mut var__vector_1: DbRef = var_v;
    let mut var_i__index: i32 = -1_i32;
    loop { //For loop_3
      let mut var_i: i32 = { //iter next_4: integer
        var_i__index = external::op_add_int((var_i__index), (1_i32));
        {let db = (vector::get_vector(&(var__vector_1), u32::from((4_i32)), (var_i__index), &s.database.allocations)); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((0_i32)))} }
        } /*iter next_4: integer*/;
      if !(external::op_conv_bool_from_int((var_i))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        var_t = external::op_add_int((var_t), (var_i));
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  var_t
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    let _pre13 = { //Vector_3: vector<integer>["__ref_1"]
      OpDatabase(stores, var___ref_1, 19_i32);
      let mut var__vec_1: DbRef = DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))};
      {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
      let mut var__elm_2: DbRef = OpNewRecord(stores, var__vec_1, 18_i32, 65535_i32);
      {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (1_i32));};
      OpFinishRecord(stores, var__vec_1, var__elm_2, 18_i32, 65535_i32);
      var__elm_2 = OpNewRecord(stores, var__vec_1, 18_i32, 65535_i32);
      {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (2_i32));};
      OpFinishRecord(stores, var__vec_1, var__elm_2, 18_i32, 65535_i32);
      var__elm_2 = OpNewRecord(stores, var__vec_1, 18_i32, 65535_i32);
      {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (3_i32));};
      OpFinishRecord(stores, var__vec_1, var__elm_2, 18_i32, 65535_i32);
      var__elm_2 = OpNewRecord(stores, var__vec_1, 18_i32, 65535_i32);
      {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (4_i32));};
      OpFinishRecord(stores, var__vec_1, var__elm_2, 18_i32, 65535_i32);
      var__elm_2 = OpNewRecord(stores, var__vec_1, 18_i32, 65535_i32);
      {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (5_i32));};
      OpFinishRecord(stores, var__vec_1, var__elm_2, 18_i32, 65535_i32);
      var__vec_1
      } /*Vector_3: vector<integer>["__ref_1"]*/;
    n_sum(stores, _pre13)
    } /*block_2: integer*/;
  let _pre14 = { //Formatted string_4: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 15";
    &var___work_1
    } /*Formatted string_4: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (15_i32), _pre14, "sum_vector", 6_i32);
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_sum_vector() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
