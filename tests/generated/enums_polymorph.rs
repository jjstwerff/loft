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
    let e = db.enumerate("Value");
    db.value(e, "Integer", u16::MAX);
    db.value(e, "Text", u16::MAX);
    db.value(e, "Array", u16::MAX);
    db.value(e, "add", u16::MAX);
    let s = db.structure("Integer", 1); // 20
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 0);
    let s = db.structure("Text", 2); // 21
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 5);
    let s = db.structure("Array", 3); // 22
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let vec_v = db.vector(0);
    db.field(s, "v", vec_v);
    let s = db.structure("main_vector<Value>", 0); // 23
    let vec_vector = db.vector(19);
    db.field(s, "vector", vec_vector);
    db.vector(19);
    db.finish();
}

fn t_7Integer_add(stores: &mut Stores, mut var_self: DbRef) -> i32 { //block_1: integer
  {let db = (var_self); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} }
  } /*block_1: integer*/

fn t_4Text_add(stores: &mut Stores, mut var_self: DbRef) -> i32 { //block_1: integer
  ({let db = (var_self); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((4_i32))) as u32))}).parse().unwrap_or(i32::MIN)
  } /*block_1: integer*/

fn t_5Array_add(stores: &mut Stores, mut var_self: DbRef) -> i32 { //block_1: integer
  let mut var_n: i32 = 0_i32;
  { //For block_2: void
    let mut var__vector_1: DbRef = DbRef {store_nr: (var_self).store_nr, rec: (var_self).rec, pos: (var_self).pos + u32::from((4_i32))};
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
        var_n = external::op_add_int((var_n), (var_v));
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  var_n
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    OpDatabase(stores, var___ref_1, 23_i32);
    let mut var_l: DbRef = DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))};
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var_l, 24_i32, 65535_i32);
    {let db = (var__elm_1); let s_val = ("123").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((2_u8)));};
    OpFinishRecord(stores, var_l, var__elm_1, 24_i32, 65535_i32);
    var__elm_1 = OpNewRecord(stores, var_l, 24_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (101_i32));};
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((1_u8)));};
    OpFinishRecord(stores, var_l, var__elm_1, 24_i32, 65535_i32);
    var__elm_1 = OpNewRecord(stores, var_l, 24_i32, 65535_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (0_i32));};
    let mut var__elm_2: DbRef = OpNewRecord(stores, var__elm_1, 22_i32, 1_i32);
    {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (1_i32));};
    OpFinishRecord(stores, var__elm_1, var__elm_2, 22_i32, 1_i32);
    var__elm_2 = OpNewRecord(stores, var__elm_1, 22_i32, 1_i32);
    {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (2_i32));};
    OpFinishRecord(stores, var__elm_1, var__elm_2, 22_i32, 1_i32);
    var__elm_2 = OpNewRecord(stores, var__elm_1, 22_i32, 1_i32);
    {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (3_i32));};
    OpFinishRecord(stores, var__elm_1, var__elm_2, 22_i32, 1_i32);
    var__elm_2 = OpNewRecord(stores, var__elm_1, 22_i32, 1_i32);
    {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (4_i32));};
    OpFinishRecord(stores, var__elm_1, var__elm_2, 22_i32, 1_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((3_u8)));};
    OpFinishRecord(stores, var_l, var__elm_1, 24_i32, 65535_i32);
    let mut var_c: i32 = 0_i32;
    { //For block_3: void
      let mut var__vector_3: DbRef = var_l;
      let mut var_v__index: i32 = -1_i32;
      loop { //For loop_4
        let mut var_v: DbRef = { //iter next_5: ref(Value)
          var_v__index = external::op_add_int((var_v__index), (1_i32));
          DbRef {store_nr: (vector::get_vector(&(var__vector_3), u32::from((8_i32)), (var_v__index), &s.database.allocations)).store_nr, rec: (vector::get_vector(&(var__vector_3), u32::from((8_i32)), (var_v__index), &s.database.allocations)).rec, pos: (vector::get_vector(&(var__vector_3), u32::from((8_i32)), (var_v__index), &s.database.allocations)).pos + u32::from((0_i32))}
          } /*iter next_5: ref(Value)*/;
        if !((var_v).rec != 0) { //break_6: void
          break;
          } /*break_6: void*/ else {()};
        { //block_7: void
          let mut var_a: i32 = t_5Value_add(stores, var_v);
          if external::op_conv_bool_from_int((var_a)) { //block_8: void
            var_c = external::op_add_int((var_c), (var_a));
            } /*block_8: void*/ else {()};
          } /*block_7: void*/;
        } /*For loop_4*/;
      } /*For block_3: void*/;
    let mut var_t: DbRef = if (if ({let db = (DbRef {store_nr: (vector::get_vector(&(var_l), u32::from((8_i32)), (1_i32), &s.database.allocations)).store_nr, rec: (vector::get_vector(&(var_l), u32::from((8_i32)), (1_i32), &s.database.allocations)).rec, pos: (vector::get_vector(&(var_l), u32::from((8_i32)), (1_i32), &s.database.allocations)).pos + u32::from((0_i32))}); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), 0) as u8}) == 255 { i32::MIN } else { i32::from(({let db = (DbRef {store_nr: (vector::get_vector(&(var_l), u32::from((8_i32)), (1_i32), &s.database.allocations)).store_nr, rec: (vector::get_vector(&(var_l), u32::from((8_i32)), (1_i32), &s.database.allocations)).rec, pos: (vector::get_vector(&(var_l), u32::from((8_i32)), (1_i32), &s.database.allocations)).pos + u32::from((0_i32))}); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), 0) as u8})) }) == (1_i32) {DbRef {store_nr: (vector::get_vector(&(var_l), u32::from((8_i32)), (1_i32), &s.database.allocations)).store_nr, rec: (vector::get_vector(&(var_l), u32::from((8_i32)), (1_i32), &s.database.allocations)).rec, pos: (vector::get_vector(&(var_l), u32::from((8_i32)), (1_i32), &s.database.allocations)).pos + u32::from((0_i32))}} else {stores.null()};
    { //Formatted string_9: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_l, 24_i32, 0_i32);
      var___work_1 += ":";
      ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_c)), 10 as u8, 0_i32, 32 as u8, false, false);
      var___work_1 += " ";
      ops::format_long(&mut var___work_1, external::op_conv_long_from_int(({let db = (var_t); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} })), 10 as u8, 0_i32, 32 as u8, false, false);
      &var___work_1
      } /*Formatted string_9: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_10: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"[Text {v:\"123\"},Integer {v:101},Array {v:[1,2,3,4]}]:234 101\"";
    &var___work_2
    } /*Formatted string_10: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("[Text {v:\"123\"},Integer {v:101},Array {v:[1,2,3,4]}]:234 101"), _pre13, "polymorph", 34_i32);
  ;
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

fn t_5Value_add(stores: &mut Stores, mut var_self: DbRef) -> i32 { //dynamic_fn_1: integer
  if (if ({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), 0) as u8}) == 255 { i32::MIN } else { i32::from(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), 0) as u8})) }) == (1_i32) { //ret_2: void
    return t_7Integer_add(stores, var_self);
    } /*ret_2: void*/ else {()};
  if (if ({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), 0) as u8}) == 255 { i32::MIN } else { i32::from(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), 0) as u8})) }) == (2_i32) { //ret_3: void
    return t_4Text_add(stores, var_self);
    } /*ret_3: void*/ else {()};
  if (if ({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), 0) as u8}) == 255 { i32::MIN } else { i32::from(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), 0) as u8})) }) == (3_i32) { //ret_4: void
    return t_5Array_add(stores, var_self);
    } /*ret_4: void*/ else {()};
  } /*dynamic_fn_1: integer*/

#[test]
fn code_polymorph() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
