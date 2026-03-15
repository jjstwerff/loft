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

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    OpDatabase(stores, var___ref_1, 19_i32);
    let mut var_a: DbRef = DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))};
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    { //For block_3: void
      let mut var_v__index: i32 = i32::MIN;
      loop { //For loop_4
        let mut var_v: i32 = { //Iter range_5: integer
          var_v__index = if !(external::op_conv_bool_from_int((var_v__index))) {1_i32} else {external::op_add_int((var_v__index), (1_i32))};
          if (400_i32) <= (var_v__index) {break} else {()};
          var_v__index
          } /*Iter range_5: integer*/;
        { //block_6: void
          let mut var__elm_1: DbRef = OpNewRecord(stores, var_a, 18_i32, 65535_i32);
          {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (external::op_mul_int((var_v), (10_i32))));};
          OpFinishRecord(stores, var_a, var__elm_1, 18_i32, 65535_i32);
          } /*block_6: void*/;
        } /*For loop_4*/;
      } /*For block_3: void*/;
    let mut var_sum: i32 = 0_i32;
    { //For block_7: void
      let mut var__vector_2: DbRef = var_a;
      let mut var_elm__index: i32 = -1_i32;
      loop { //For loop_8
        let mut var_elm: i32 = { //iter next_9: integer
          var_elm__index = external::op_add_int((var_elm__index), (1_i32));
          {let db = (vector::get_vector(&(var__vector_2), u32::from((4_i32)), (var_elm__index), &s.database.allocations)); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((0_i32)))} }
          } /*iter next_9: integer*/;
        if !(external::op_conv_bool_from_int((var_elm))) { //break_10: void
          break;
          } /*break_10: void*/ else {()};
        { //block_11: void
          var_sum = external::op_add_int((var_sum), (var_elm));
          } /*block_11: void*/;
        } /*For loop_8*/;
      } /*For block_7: void*/;
    { //Formatted string_12: text["__work_1"]
      var___work_1 = "".to_string();
      ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_sum)), 10 as u8, 0_i32, 32 as u8, false, false);
      &var___work_1
      } /*Formatted string_12: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_13: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"798000\"";
    &var___work_2
    } /*Formatted string_13: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("798000"), _pre13, "growing_vector", 8_i32);
  ;
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_growing_vector() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
