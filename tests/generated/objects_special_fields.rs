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
    let e = db.enumerate("Gender");
    db.value(e, "Male", u16::MAX);
    db.value(e, "Female", u16::MAX);
    db.value(e, "Fluid", u16::MAX);
    let s = db.structure("Object", 0); // 20
    let vec_a = db.vector(0);
    db.field(s, "a", vec_a);
    db.field(s, "b", 19);
    db.finish();
}

fn n_sum(stores: &mut Stores, mut var_o: DbRef) -> i32 { //block_1: integer
  let mut var_r: i32 = 0_i32;
  { //For block_2: void
    let mut var__vector_1: DbRef = DbRef {store_nr: (var_o).store_nr, rec: (var_o).rec, pos: (var_o).pos + u32::from((0_i32))};
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
        var_r = external::op_add_int((var_r), (var_v));
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  var_r
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    let mut var_o: DbRef = stores.null();
    OpDatabase(stores, var_o, 20_i32);
    {let db = (var_o); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var_o, 20_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (1_i32));};
    OpFinishRecord(stores, var_o, var__elm_1, 20_i32, 0_i32);
    var__elm_1 = OpNewRecord(stores, var_o, 20_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (4_i32));};
    OpFinishRecord(stores, var_o, var__elm_1, 20_i32, 0_i32);
    var__elm_1 = OpNewRecord(stores, var_o, 20_i32, 0_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (3_i32));};
    OpFinishRecord(stores, var_o, var__elm_1, 20_i32, 0_i32);
    {let db = (var_o); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((4_i32)), 0, i32::from((3_u8)));};
    let mut var__elm_2: DbRef = OpNewRecord(stores, var_o, 20_i32, 0_i32);
    let _pre13 = n_sum(stores, var_o);
    {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (_pre13));};
    OpFinishRecord(stores, var_o, var__elm_2, 20_i32, 0_i32);
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_o, 20_i32, 0_i32);
      OpFreeRef(stores, var_o);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre14 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"{a:[1,4,3,8],b:Fluid}\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("{a:[1,4,3,8],b:Fluid}"), _pre14, "special_fields", 14_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_special_fields() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
