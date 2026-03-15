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
    let s = db.structure("Point", 0); // 18
    let byte_r = db.byte(0, false);
    db.field(s, "r", byte_r);
    let byte_g = db.byte(0, false);
    db.field(s, "g", byte_g);
    let byte_b = db.byte(0, false);
    db.field(s, "b", byte_b);
    let s = db.structure("main_vector<Point>", 0); // 19
    let vec_vector = db.vector(18);
    db.field(s, "vector", vec_vector);
    db.vector(18);
    db.finish();
}

fn t_5Point_value(stores: &mut Stores, mut var_self: DbRef) -> i32 { //block_1: integer
  external::op_add_int((external::op_add_int((external::op_mul_int(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), i32::from((0_i32)))}), (65536_i32))), (external::op_mul_int(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((1_i32)), i32::from((0_i32)))}), (256_i32))))), ({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((2_i32)), i32::from((0_i32)))}))
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    OpDatabase(stores, var___ref_1, 19_i32);
    let mut var_points: DbRef = DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))};
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var_points, 20_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), i32::from((0_i32)), (128_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((2_i32)), i32::from((0_i32)), (128_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((1_i32)), i32::from((0_i32)), (0_i32));};
    OpFinishRecord(stores, var_points, var__elm_1, 20_i32, 65535_i32);
    var__elm_1 = OpNewRecord(stores, var_points, 20_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((2_i32)), i32::from((0_i32)), (255_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), i32::from((0_i32)), (0_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((1_i32)), i32::from((0_i32)), (0_i32));};
    OpFinishRecord(stores, var_points, var__elm_1, 20_i32, 65535_i32);
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "size:".to_string();
      ops::format_long(&mut var___work_1, external::op_conv_long_from_int((3_i32)), 10 as u8, 0_i32, 32 as u8, false, false);
      var___work_1 += " purple:";
      OpFormatDatabase(stores, &var___work_1, vector::get_vector(&(var_points), u32::from((3_i32)), (0_i32), &s.database.allocations), 18_i32, 0_i32);
      var___work_1 += " value:";
      let _pre13 = external::op_conv_long_from_int((t_5Point_value(stores, vector::get_vector(&(var_points), u32::from((3_i32)), (0_i32), &s.database.allocations))));
      ops::format_long(&mut var___work_1, _pre13, 16 as u8, 0_i32, 32 as u8, false, false);
      var___work_1 += " blue:";
      OpFormatDatabase(stores, &var___work_1, vector::get_vector(&(var_points), u32::from((3_i32)), (1_i32), &s.database.allocations), 18_i32, 0_i32);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre14 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"size:3 purple:{r:128,g:0,b:128} value:800080 blue:{r:0,g:0,b:255}\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("size:3 purple:{r:128,g:0,b:128} value:800080 blue:{r:0,g:0,b:255}"), _pre14, "colours", 15_i32);
  ;
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_colours() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
