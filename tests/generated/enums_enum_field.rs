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
    let e = db.enumerate("Content");
    db.value(e, "Long", u16::MAX);
    db.value(e, "Float", u16::MAX);
    db.value(e, "Single", u16::MAX);
    db.value(e, "Text", u16::MAX);
    let s = db.structure("Long", 1); // 19
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 1);
    let s = db.structure("Float", 2); // 20
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 3);
    let s = db.structure("Single", 3); // 21
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 2);
    let s = db.structure("Text", 4); // 22
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "v", 5);
    let s = db.structure("Container", 0); // 23
    db.field(s, "name", 5);
    db.field(s, "content", 18);
    let vec_list = db.vector(18);
    db.field(s, "list", vec_list);
    db.vector(18);
    db.finish();
}

fn n_fill(stores: &mut Stores) -> DbRef { //block_1: ref(Container)
  let mut var___ref_1: DbRef = stores.null();
  { //Object_2: ref(Container)["__ref_1"]
    OpDatabase(stores, var___ref_1, 23_i32);
    {let db = (var___ref_1); let s_val = ("testing").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((16_i32)), s_pos as i32);};
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (0_i32));};
    {let db = (DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))}); stores.store_mut(&db).set_single(db.rec, db.pos + u32::from((4_i32)), (1234.56_f32));};
    {let db = (DbRef {store_nr: (var___ref_1).store_nr, rec: (var___ref_1).rec, pos: (var___ref_1).pos + u32::from((0_i32))}); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((3_u8)));};
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((20_i32)), (0_i32));};
    let mut var__elm_1: DbRef = OpNewRecord(stores, var___ref_1, 23_i32, 2_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_long(db.rec, db.pos + u32::from((8_i32)), (9876543210_i64));};
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((1_u8)));};
    OpFinishRecord(stores, var___ref_1, var__elm_1, 23_i32, 2_i32);
    var__elm_1 = OpNewRecord(stores, var___ref_1, 23_i32, 2_i32);
    {let db = (var__elm_1); let s_val = ("An example sentence of text").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((4_u8)));};
    OpFinishRecord(stores, var___ref_1, var__elm_1, 23_i32, 2_i32);
    var__elm_1 = OpNewRecord(stores, var___ref_1, 23_i32, 2_i32);
    {let db = (var__elm_1); stores.store_mut(&db).set_float(db.rec, db.pos + u32::from((8_i32)), (3.141592653589793_f64));};
    {let db = (var__elm_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((0_i32)), 0, i32::from((2_u8)));};
    OpFinishRecord(stores, var___ref_1, var__elm_1, 23_i32, 2_i32);
    var___ref_1
    } /*Object_2: ref(Container)["__ref_1"]*/
  } /*block_1: ref(Container)*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    let mut var_c: DbRef = n_fill(stores);
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "Container: ".to_string();
      OpFormatDatabase(stores, &var___work_1, var_c, 23_i32, 1_i32);
      OpFreeRef(stores, var_c);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"Container: { name: \"testing\",\n  content: Single { v: 1234.56 },\n  list: [ Long { v: 9876543210 }, Text { v: \"An example sentence of text\" }, Float { v: 3.141592653589793 } ]\n}\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("Container: { name: \"testing\",\n  content: Single { v: 1234.56 },\n  list: [ Long { v: 9876543210 }, Text { v: \"An example sentence of text\" }, Float { v: 3.141592653589793 } ]\n}"), _pre13, "enum_field", 29_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_enum_field() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
