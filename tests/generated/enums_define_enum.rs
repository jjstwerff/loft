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
    let e = db.enumerate("Code");
    db.value(e, "Null", u16::MAX);
    db.value(e, "Line", u16::MAX);
    db.value(e, "Integer", u16::MAX);
    db.value(e, "Enum", u16::MAX);
    db.value(e, "Boolean", u16::MAX);
    db.value(e, "Float", u16::MAX);
    db.value(e, "Text", u16::MAX);
    db.value(e, "Call", u16::MAX);
    db.value(e, "Block", u16::MAX);
    db.value(e, "Loop", u16::MAX);
    db.value(e, "Continue", u16::MAX);
    db.value(e, "Break", u16::MAX);
    db.value(e, "Return", u16::MAX);
    db.value(e, "Set", u16::MAX);
    db.value(e, "Var", u16::MAX);
    db.value(e, "If", u16::MAX);
    db.value(e, "Drop", u16::MAX);
    let s = db.structure("Line", 2); // 19
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "line", 0);
    let s = db.structure("Integer", 3); // 20
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "i_value", 0);
    let s = db.structure("Enum", 4); // 21
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let byte_e_value = db.byte(0, true);
    db.field(s, "e_value", byte_e_value);
    let short_tp = db.short(0, true);
    db.field(s, "tp", short_tp);
    let s = db.structure("Boolean", 5); // 24
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "b_value", 4);
    let s = db.structure("Float", 6); // 25
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "f_value", 3);
    let s = db.structure("Text", 7); // 26
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "t_value", 5);
    let s = db.structure("Call", 8); // 27
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "function", 5);
    let byte_parameters = db.byte(0, true);
    db.field(s, "parameters", byte_parameters);
    let s = db.structure("Block", 9); // 28
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "name", 5);
    let short_tp = db.short(0, true);
    db.field(s, "tp", short_tp);
    let short_size = db.short(0, true);
    db.field(s, "size", short_size);
    let s = db.structure("Loop", 10); // 29
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    db.field(s, "name", 5);
    let short_tp = db.short(0, true);
    db.field(s, "tp", short_tp);
    let short_size = db.short(0, true);
    db.field(s, "size", short_size);
    let s = db.structure("Continue", 11); // 30
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let byte_loops = db.byte(0, true);
    db.field(s, "loops", byte_loops);
    let s = db.structure("Break", 12); // 31
    let byte_enum = db.byte(0, false);
    db.field(s, "enum", byte_enum);
    let byte_loops = db.byte(0, true);
    db.field(s, "loops", byte_loops);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    let mut var_v: DbRef = s.db_from_text(("Call { function: \"foo\", parameters: 2 }"), (18_i32));
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_v, 18_i32, 0_i32);
      OpFreeRef(stores, var_v);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"Call {function:\"foo\",parameters:2}\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("Call {function:\"foo\",parameters:2}"), _pre13, "define_enum", 23_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_define_enum() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
