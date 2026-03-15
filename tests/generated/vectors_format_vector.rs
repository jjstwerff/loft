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
    let mut var_v: DbRef = DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))};
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var_v, 18_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (1_i32));};
    OpFinishRecord(stores, var_v, var__elm_1, 18_i32, 65535_i32);
    var__elm_1 = OpNewRecord(stores, var_v, 18_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (2_i32));};
    OpFinishRecord(stores, var_v, var__elm_1, 18_i32, 65535_i32);
    var__elm_1 = OpNewRecord(stores, var_v, 18_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (4_i32));};
    OpFinishRecord(stores, var_v, var__elm_1, 18_i32, 65535_i32);
    var__elm_1 = OpNewRecord(stores, var_v, 18_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (8_i32));};
    OpFinishRecord(stores, var_v, var__elm_1, 18_i32, 65535_i32);
    let mut var__elm_2: DbRef = OpNewRecord(stores, var_v, 18_i32, 65535_i32);
    {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (16_i32));};
    OpFinishRecord(stores, var_v, var__elm_2, 18_i32, 65535_i32);
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_v, 18_i32, 0_i32);
      var___work_1 += " ";
      let _pre13 = external::op_conv_long_from_int((t_6vector_len(stores, var_v)));
      ops::format_long(&mut var___work_1, _pre13, 10 as u8, 0_i32, 32 as u8, false, false);
      var___work_1 += " ";
      ops::format_long(&mut var___work_1, external::op_conv_long_from_int(({let db = (vector::get_vector(&(var_v), u32::from((4_i32)), (2_i32), &s.database.allocations)); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((0_i32)))} })), 10 as u8, 0_i32, 32 as u8, false, false);
      var___work_1 += " ";
      var___work_1 += "[";
      let mut var__index_3: i32 = i32::MIN;
      let mut var__count_4: i32 = 0_i32;
      loop { //Append Iter_4
        let mut var__val_5: i32 = { //Vector Index_5: integer
          let _pre15 = { //Iter range_6: integer
            var__index_3 = if !(external::op_conv_bool_from_int((var__index_3))) {1_i32} else {external::op_add_int((var__index_3), (1_i32))};
            if (3_i32) <= (var__index_3) {break} else {()};
            var__index_3
            } /*Iter range_6: integer*/;
          let _pre14 = vector::get_vector(&(var_v), u32::from((4_i32)), (_pre15), &s.database.allocations);
          {let db = (_pre14); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((0_i32)))} }
          } /*Vector Index_5: integer*/;
        if (0_i32) < (var__count_4) {var___work_1 += ","} else {()};
        var__count_4 = external::op_add_int((var__count_4), (1_i32));
        ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var__val_5)), 10 as u8, 0_i32, 32 as u8, false, false);
        } /*Append Iter_4*/;
      var___work_1 += "]";
      var___work_1 += " ";
      var___work_1 += "[";
      let mut var__index_6: i32 = i32::MIN;
      let mut var__count_7: i32 = 0_i32;
      loop { //Append Iter_7
        let mut var__val_8: i32 = { //Vector Index_8: integer
          let _pre17 = { //Iter range_9: integer
            var__index_6 = if !(external::op_conv_bool_from_int((var__index_6))) {3_i32} else {external::op_min_int((var__index_6), (1_i32))};
            if (var__index_6) < (1_i32) {break} else {()};
            var__index_6
            } /*Iter range_9: integer*/;
          let _pre16 = vector::get_vector(&(var_v), u32::from((4_i32)), (_pre17), &s.database.allocations);
          {let db = (_pre16); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((0_i32)))} }
          } /*Vector Index_8: integer*/;
        if (0_i32) < (var__count_7) {var___work_1 += ","} else {()};
        var__count_7 = external::op_add_int((var__count_7), (1_i32));
        ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var__val_8)), 10 as u8, 0_i32, 32 as u8, false, false);
        } /*Append Iter_7*/;
      var___work_1 += "]";
      var___work_1 += " ";
      var___work_1 += "[";
      let mut var_x__index: i32 = -1_i32;
      let mut var_x__count: i32 = 0_i32;
      loop { //Append Iter_10
        let mut var__val_9: i32 = { //Iter For_11: integer
          let mut var_x: i32 = { //iter next_12: integer
            var_x__index = external::op_add_int((var_x__index), (1_i32));
            {let db = (vector::get_vector(&(var_v), u32::from((4_i32)), (var_x__index), &s.database.allocations)); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((0_i32)))} }
            } /*iter next_12: integer*/;
          if !(external::op_conv_bool_from_int((var_x))) { //break_13: void
            break;
            } /*break_13: void*/ else {()};
          if (4_i32) <= (var_x) {()} else {continue};
          { //block_14: integer
            external::op_div_int((var_x), (2_i32))
            } /*block_14: integer*/
          } /*Iter For_11: integer*/;
        if (0_i32) < (var_x__count) {var___work_1 += ","} else {()};
        var_x__count = external::op_add_int((var_x__count), (1_i32));
        ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var__val_9)), 10 as u8, 0_i32, 32 as u8, false, false);
        } /*Append Iter_10*/;
      var___work_1 += "]";
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre18 = { //Formatted string_15: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"[1,2,4,8,16] 5 4 [2,4] [8,4,2] [2,4,8]\"";
    &var___work_2
    } /*Formatted string_15: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("[1,2,4,8,16] 5 4 [2,4] [8,4,2] [2,4,8]"), _pre18, "format_vector", 6_i32);
  ;
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_format_vector() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
