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
  let mut var___ref_2: DbRef = stores.null();
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    OpDatabase(stores, var___ref_2, 19_i32);
    let mut var_v: DbRef = DbRef {store_nr: (var___ref_2).store_nr, rec: (var___ref_2).rec, pos: (var___ref_2).pos + u32::from((0_i32))};
    {let db = (var___ref_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    OpDatabase(stores, var___ref_1, 19_i32);
    var_v = DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))};
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var_n__index: i32 = i32::MIN;
    loop { //For comprehension_3
      let mut var_n: i32 = { //Iter range_4: integer
        var_n__index = if !(external::op_conv_bool_from_int((var_n__index))) {1_i32} else {external::op_add_int((var_n__index), (1_i32))};
        if (10_i32) <= (var_n__index) {break} else {()};
        var_n__index
        } /*Iter range_4: integer*/;
      if (external::op_rem_int((var_n), (2_i32))) == (0_i32) {()} else {continue};
      let mut var__comp_2: i32 = { //block_5: integer
        var_n
        } /*block_5: integer*/;
      let mut var__elm_1: DbRef = OpNewRecord(stores, var_v, 18_i32, 65535_i32);
      {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (var__comp_2));};
      OpFinishRecord(stores, var_v, var__elm_1, 18_i32, 65535_i32);
      } /*For comprehension_3*/;
    { //Formatted string_6: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_v, 18_i32, 0_i32);
      &var___work_1
      } /*Formatted string_6: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_7: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"[2,4,6,8]\"";
    &var___work_2
    } /*Formatted string_7: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("[2,4,6,8]"), _pre13, "for_comprehension_if", 4_i32);
  ;
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  OpFreeRef(stores, var___ref_2);
  } /*block_1: void*/

#[test]
fn code_for_comprehension_if() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
