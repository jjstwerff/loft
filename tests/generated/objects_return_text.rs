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
    let s = db.structure("Data", 0); // 18
    db.field(s, "name", 5);
    db.field(s, "number", 0);
    db.finish();
}

fn n_data(stores: &mut Stores, mut var_n: Str, mut var_res: DbRef) -> Str { //block_1: text["res"]
  OpDatabase(stores, var_res, 18_i32);
  {let db = (var_res); let s_val = (&var_n).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((0_i32)), s_pos as i32);};
  {let db = (var_res); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (0_i32));};
  Str::new({let db = (var_res); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((0_i32))) as u32))})
  } /*block_1: text["res"]*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__ref_1"]
    n_data(stores, "test", var___ref_1)
    } /*block_2: text["__ref_1"]*/.to_string();
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_text(&mut var___work_1, &var_test_value, 0_i32, -1, 32);
    var___work_1 += " != \"test\"";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (&var_test_value) == ("test"), _pre13, "return_text", 14_i32);
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_return_text() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
