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
    let e = db.enumerate("Value");
    db.value(e, "Null", u16::MAX);
    db.value(e, "Integer", u16::MAX);
    db.value(e, "Boolean", u16::MAX);
    db.value(e, "Float", u16::MAX);
    db.value(e, "Text", u16::MAX);
    db.value(e, "Object", u16::MAX);
    db.value(e, "Array", u16::MAX);
    let s = db.structure("Integer", 2); // 19
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "i_value", 0);
    let s = db.structure("Boolean", 3); // 20
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "b_value", 4);
    let s = db.structure("Float", 4); // 21
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "f_value", 3);
    let s = db.structure("Text", 5); // 22
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "t_value", 5);
    let s = db.structure("Object", 6); // 23
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let vec_fields = db.vector(24);
    db.field(s, "fields", vec_fields);
    let s = db.structure("Pair", 0); // 24
    let short_field = db.short(0, true);
    db.field(s, "field", short_field);
    db.field(s, "value", 18);
    db.vector(24);
    let s = db.structure("Array", 7); // 27
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let vec_content = db.vector(18);
    db.field(s, "content", vec_content);
    db.vector(18);
    let s = db.structure("Field", 0); // 29
    let short_field = db.short(0, true);
    db.field(s, "field", short_field);
    db.field(s, "name", 5);
    let s = db.structure("Json", 0); // 30
    let vec_key_fields = db.vector(29);
    db.field(s, "key_fields", vec_key_fields);
    let hash_key_hash = db.hash(29, &["name".to_string()]);
    db.field(s, "key_hash", hash_key_hash);
    db.field(s, "data", 18);
    db.vector(29);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    let mut var_v: DbRef = s.db_from_text(("{ data: Integer { i_value: 12 } }"), (30_i32));
    let mut var_i: DbRef = DbRef {store_nr: (var_v).store_nr, rec: (var_v).rec, pos: (var_v).pos + u32::from((0_i32))};
    n_assert(stores, (if ({let db = (var_i); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), 0) as u8}) == 255 { i32::MIN } else { i32::from(({let db = (var_i); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), 0) as u8})) }) == (if (2_u8) == 255 { i32::MIN } else { i32::from((2_u8)) }), "Compare", "general_json", 32_i32);
    let mut var_w: DbRef = s.db_from_text(("Text { t_value: \"Something\" }"), (18_i32));
    OpCopyRecord(stores, var_w, DbRef {store_nr: (var_v).store_nr, rec: (var_v).rec, pos: (var_v).pos + u32::from((0_i32))}, 18_i32);
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_v, 30_i32, 0_i32);
      var___work_1 += " & ";
      OpFormatDatabase(stores, &var___work_1, var_i, 18_i32, 0_i32);
      OpFreeRef(stores, var_w);
      OpFreeRef(stores, var_v);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"{key_fields:[],data:Text {t_value:\"Something\"}} & Text {t_value:\"Something\"}\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("{key_fields:[],data:Text {t_value:\"Something\"}} & Text {t_value:\"Something\"}"), _pre13, "general_json", 37_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_general_json() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
