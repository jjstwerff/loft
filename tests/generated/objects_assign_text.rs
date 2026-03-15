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
    let s = db.structure("Object", 0); // 18
    db.field(s, "a", 5);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_3: String = "".to_string();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_2"]
    let mut var_o: DbRef = stores.null();
    OpDatabase(stores, var_o, 18_i32);
    {let db = (var_o); let s_val = ("a").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    {let db = (var_o); let s_val = ("b").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    let mut var__field_1: String = {let db = (var_o); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((0_i32))) as u32))}.to_string();
    var__field_1 += "c";
    {let db = (var_o); let s_val = (&var__field_1).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    let mut var__field_2: String = {let db = (var_o); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((0_i32))) as u32))}.to_string();
    var__field_2 += "d";
    var__field_2 += "e";
    {let db = (var_o); let s_val = (&var__field_2).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    let _pre13 = { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      ops::format_text(&mut var___work_1, {let db = (var_o); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((0_i32))) as u32))}, 0_i32, -1, 32);
      var___work_1 += "f";
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/;
    {let db = (var_o); let s_val = (_pre13).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    { //Formatted string_4: text["__work_2"]
      var___work_2 = "".to_string();
      OpFormatDatabase(stores, &var___work_2, var_o, 18_i32, 0_i32);
      ;
      ;
      OpFreeRef(stores, var_o);
      &var___work_2
      } /*Formatted string_4: text["__work_2"]*/
    } /*block_2: text["__work_2"]*/.to_string();
  let _pre14 = { //Formatted string_5: text["__work_3"]
    var___work_3 = "Test failed ".to_string();
    ops::format_text(&mut var___work_3, &var_test_value, 0_i32, -1, 32);
    var___work_3 += " != \"{a:\"bcdef\"}\"";
    &var___work_3
    } /*Formatted string_5: text["__work_3"]*/;
  n_assert(stores, (&var_test_value) == ("{a:\"bcdef\"}"), _pre14, "assign_text", 11_i32);
  ;
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_assign_text() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
