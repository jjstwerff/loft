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

fn n_sum(stores: &mut Stores, mut var_r: DbRef) -> i32 { //block_1: integer
  let mut var_res: i32 = 0_i32;
  { //For block_2: void
    let mut var__vector_1: DbRef = var_r;
    let mut var_v__index: i32 = -1_i32;
    loop { //For loop_3
      let mut var_v: i32 = { //iter next_4: integer
        var_v__index = external::op_add_int((var_v__index), (1_i32));
        {let db = (vector::get_vector(&(var__vector_1), u32::from((4_i32)), (var_v__index), &s.database.allocations)); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((0_i32)))} }
        } /*iter next_4: integer*/;
      if !(external::op_conv_bool_from_int((var_v))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        var_res = external::op_add_int((var_res), (var_v));
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  var_res
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_4: DbRef = stores.null();
  let mut var___ref_3: DbRef = stores.null();
  let mut var___ref_2: DbRef = stores.null();
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    let _pre14 = { //Vector_3: vector<integer>["__ref_1"]
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
    let _pre13 = n_sum(stores, _pre14);
    let _pre16 = { //Append Vector_4: vector<integer>["__ref_4"]
      OpDatabase(stores, var___ref_4, 19_i32);
      let mut var__vec_7: DbRef = DbRef {store_nr: (var___ref_4).store_nr, rec: (var___ref_4).rec, pos: (var___ref_4).pos + u32::from((0_i32))};
      {let db = (var___ref_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
      let _pre17 = { //Vector_5: vector<integer>["__ref_2"]
        OpDatabase(stores, var___ref_2, 19_i32);
        let mut var__vec_3: DbRef = DbRef {store_nr: (var___ref_2).store_nr, rec: (var___ref_2).rec, pos: (var___ref_2).pos + u32::from((0_i32))};
        {let db = (var___ref_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
        let mut var__elm_4: DbRef = OpNewRecord(stores, var__vec_3, 18_i32, 65535_i32);
        {let db = (var__elm_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (1_i32));};
        OpFinishRecord(stores, var__vec_3, var__elm_4, 18_i32, 65535_i32);
        var__elm_4 = OpNewRecord(stores, var__vec_3, 18_i32, 65535_i32);
        {let db = (var__elm_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (2_i32));};
        OpFinishRecord(stores, var__vec_3, var__elm_4, 18_i32, 65535_i32);
        var__elm_4 = OpNewRecord(stores, var__vec_3, 18_i32, 65535_i32);
        {let db = (var__elm_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (3_i32));};
        OpFinishRecord(stores, var__vec_3, var__elm_4, 18_i32, 65535_i32);
        var__vec_3
        } /*Vector_5: vector<integer>["__ref_2"]*/;
      s.database.vector_add(&(var__vec_7), &(_pre17), (0_i32));;
      let _pre18 = { //Vector_6: vector<integer>["__ref_3"]
        OpDatabase(stores, var___ref_3, 19_i32);
        let mut var__vec_5: DbRef = DbRef {store_nr: (var___ref_3).store_nr, rec: (var___ref_3).rec, pos: (var___ref_3).pos + u32::from((0_i32))};
        {let db = (var___ref_3); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
        let mut var__elm_6: DbRef = OpNewRecord(stores, var__vec_5, 18_i32, 65535_i32);
        {let db = (var__elm_6); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (4_i32));};
        OpFinishRecord(stores, var__vec_5, var__elm_6, 18_i32, 65535_i32);
        var__elm_6 = OpNewRecord(stores, var__vec_5, 18_i32, 65535_i32);
        {let db = (var__elm_6); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (5_i32));};
        OpFinishRecord(stores, var__vec_5, var__elm_6, 18_i32, 65535_i32);
        var__vec_5
        } /*Vector_6: vector<integer>["__ref_3"]*/;
      s.database.vector_add(&(var__vec_7), &(_pre18), (0_i32));;
      var__vec_7
      } /*Append Vector_4: vector<integer>["__ref_4"]*/;
    let _pre15 = external::op_mul_int((100_i32), (n_sum(stores, { //Append Vector_4: vector<integer>["__ref_4"]
      OpDatabase(stores, var___ref_4, 19_i32);
      let mut var__vec_7: DbRef = DbRef {store_nr: (var___ref_4).store_nr, rec: (var___ref_4).rec, pos: (var___ref_4).pos + u32::from((0_i32))};
      {let db = (var___ref_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
      let _pre19 = { //Vector_5: vector<integer>["__ref_2"]
        OpDatabase(stores, var___ref_2, 19_i32);
        let mut var__vec_3: DbRef = DbRef {store_nr: (var___ref_2).store_nr, rec: (var___ref_2).rec, pos: (var___ref_2).pos + u32::from((0_i32))};
        {let db = (var___ref_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
        let mut var__elm_4: DbRef = OpNewRecord(stores, var__vec_3, 18_i32, 65535_i32);
        {let db = (var__elm_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (1_i32));};
        OpFinishRecord(stores, var__vec_3, var__elm_4, 18_i32, 65535_i32);
        var__elm_4 = OpNewRecord(stores, var__vec_3, 18_i32, 65535_i32);
        {let db = (var__elm_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (2_i32));};
        OpFinishRecord(stores, var__vec_3, var__elm_4, 18_i32, 65535_i32);
        var__elm_4 = OpNewRecord(stores, var__vec_3, 18_i32, 65535_i32);
        {let db = (var__elm_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (3_i32));};
        OpFinishRecord(stores, var__vec_3, var__elm_4, 18_i32, 65535_i32);
        var__vec_3
        } /*Vector_5: vector<integer>["__ref_2"]*/;
      s.database.vector_add(&(var__vec_7), &(_pre19), (0_i32));;
      let _pre20 = { //Vector_6: vector<integer>["__ref_3"]
        OpDatabase(stores, var___ref_3, 19_i32);
        let mut var__vec_5: DbRef = DbRef {store_nr: (var___ref_3).store_nr, rec: (var___ref_3).rec, pos: (var___ref_3).pos + u32::from((0_i32))};
        {let db = (var___ref_3); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
        let mut var__elm_6: DbRef = OpNewRecord(stores, var__vec_5, 18_i32, 65535_i32);
        {let db = (var__elm_6); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (4_i32));};
        OpFinishRecord(stores, var__vec_5, var__elm_6, 18_i32, 65535_i32);
        var__elm_6 = OpNewRecord(stores, var__vec_5, 18_i32, 65535_i32);
        {let db = (var__elm_6); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (5_i32));};
        OpFinishRecord(stores, var__vec_5, var__elm_6, 18_i32, 65535_i32);
        var__vec_5
        } /*Vector_6: vector<integer>["__ref_3"]*/;
      s.database.vector_add(&(var__vec_7), &(_pre20), (0_i32));;
      var__vec_7
      } /*Append Vector_4: vector<integer>["__ref_4"]*/)));
    external::op_add_int((_pre13), (external::op_mul_int((100_i32), (n_sum(stores, { //Append Vector_4: vector<integer>["__ref_4"]
      OpDatabase(stores, var___ref_4, 19_i32);
      let mut var__vec_7: DbRef = DbRef {store_nr: (var___ref_4).store_nr, rec: (var___ref_4).rec, pos: (var___ref_4).pos + u32::from((0_i32))};
      {let db = (var___ref_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
      let _pre21 = { //Vector_5: vector<integer>["__ref_2"]
        OpDatabase(stores, var___ref_2, 19_i32);
        let mut var__vec_3: DbRef = DbRef {store_nr: (var___ref_2).store_nr, rec: (var___ref_2).rec, pos: (var___ref_2).pos + u32::from((0_i32))};
        {let db = (var___ref_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
        let mut var__elm_4: DbRef = OpNewRecord(stores, var__vec_3, 18_i32, 65535_i32);
        {let db = (var__elm_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (1_i32));};
        OpFinishRecord(stores, var__vec_3, var__elm_4, 18_i32, 65535_i32);
        var__elm_4 = OpNewRecord(stores, var__vec_3, 18_i32, 65535_i32);
        {let db = (var__elm_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (2_i32));};
        OpFinishRecord(stores, var__vec_3, var__elm_4, 18_i32, 65535_i32);
        var__elm_4 = OpNewRecord(stores, var__vec_3, 18_i32, 65535_i32);
        {let db = (var__elm_4); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (3_i32));};
        OpFinishRecord(stores, var__vec_3, var__elm_4, 18_i32, 65535_i32);
        var__vec_3
        } /*Vector_5: vector<integer>["__ref_2"]*/;
      s.database.vector_add(&(var__vec_7), &(_pre21), (0_i32));;
      let _pre22 = { //Vector_6: vector<integer>["__ref_3"]
        OpDatabase(stores, var___ref_3, 19_i32);
        let mut var__vec_5: DbRef = DbRef {store_nr: (var___ref_3).store_nr, rec: (var___ref_3).rec, pos: (var___ref_3).pos + u32::from((0_i32))};
        {let db = (var___ref_3); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
        let mut var__elm_6: DbRef = OpNewRecord(stores, var__vec_5, 18_i32, 65535_i32);
        {let db = (var__elm_6); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (4_i32));};
        OpFinishRecord(stores, var__vec_5, var__elm_6, 18_i32, 65535_i32);
        var__elm_6 = OpNewRecord(stores, var__vec_5, 18_i32, 65535_i32);
        {let db = (var__elm_6); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (5_i32));};
        OpFinishRecord(stores, var__vec_5, var__elm_6, 18_i32, 65535_i32);
        var__vec_5
        } /*Vector_6: vector<integer>["__ref_3"]*/;
      s.database.vector_add(&(var__vec_7), &(_pre22), (0_i32));;
      var__vec_7
      } /*Append Vector_4: vector<integer>["__ref_4"]*/)))))
    } /*block_2: integer*/;
  let _pre23 = { //Formatted string_7: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 1515";
    &var___work_1
    } /*Formatted string_7: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (1515_i32), _pre23, "vector_argument", 13_i32);
  ;
  OpFreeRef(stores, var___ref_1);
  OpFreeRef(stores, var___ref_2);
  OpFreeRef(stores, var___ref_3);
  OpFreeRef(stores, var___ref_4);
  } /*block_1: void*/

#[test]
fn code_vector_argument() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
