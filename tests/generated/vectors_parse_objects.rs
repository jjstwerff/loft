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
    let s = db.structure("Elm", 0); // 18
    db.field(s, "n", 5);
    db.field(s, "c", 0);
    let s = db.structure("main_vector<Elm>", 0); // 19
    let vec_vector = db.vector(18);
    db.field(s, "vector", vec_vector);
    db.vector(18);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    let mut var_v: DbRef = s.db_from_text(("[ {n:'hi', c:10 }, {n:'world', c:2 } ]"), (20_i32));
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_v, 20_i32, 0_i32);
      OpFreeRef(stores, var_v);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"[{n:\"hi\",c:10},{n:\"world\",c:2}]\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("[{n:\"hi\",c:10},{n:\"world\",c:2}]"), _pre13, "parse_objects", 6_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_parse_objects() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
