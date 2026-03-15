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
    let s = db.structure("T", 0); // 18
    db.field(s, "name", 5);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_a: DbRef = stores.null();
  let mut var_test_value = { //block_2: text["a"]
    OpDatabase(stores, var_a, 18_i32);
    {let db = (var_a); let s_val = ("hello").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    let mut var_b: DbRef = var_a;
    let mut var__field_1: String = {let db = (var_b); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((0_i32))) as u32))}.to_string();
    var__field_1 += " world";
    {let db = (var_b); let s_val = (&var__field_1).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
    let _ret = {let db = (var_a); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((0_i32))) as u32))};
    ;
    OpFreeRef(stores, var_b);
    _ret
    } /*block_2: text["a"]*/.to_string();
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_text(&mut var___work_1, &var_test_value, 0_i32, -1, 32);
    var___work_1 += " != \"hello\"";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (&var_test_value) == ("hello"), _pre13, "independent_strings", 9_i32);
  ;
  OpFreeRef(stores, var_a);
  ;
  } /*block_1: void*/

#[test]
fn code_independent_strings() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
