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
    db.finish();
}

fn n_fill(stores: &mut Stores, mut var_result: DbRef) -> DbRef { //block_1: vector<text>["result"]
  vector::clear_vector(&(var_result), &mut s.database.allocations);;
  let mut var__elm_1: DbRef = OpNewRecord(stores, var_result, 7_i32, 65535_i32);
  {let db = (var__elm_1); let s_val = ("aa").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  OpFinishRecord(stores, var_result, var__elm_1, 7_i32, 65535_i32);
  let mut var__elm_2: DbRef = OpNewRecord(stores, var_result, 7_i32, 65535_i32);
  {let db = (var__elm_2); let s_val = ("bb").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  OpFinishRecord(stores, var_result, var__elm_2, 7_i32, 65535_i32);
  var_result
  } /*block_1: vector<text>["result"]*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    let mut var_t: DbRef = n_fill(stores, var___ref_1);
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_t, 7_i32, 0_i32);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"[\"aa\",\"bb\"]\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("[\"aa\",\"bb\"]"), _pre13, "fill_result", 11_i32);
  ;
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_fill_result() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
