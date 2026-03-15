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
    let s = db.structure("Pt", 0); // 18
    db.field(s, "x", 0);
    db.field(s, "y", 0);
    db.finish();
}

fn t_2Pt_dist2(stores: &mut Stores, mut var_self: DbRef) -> i32 { //block_1: integer
  external::op_add_int((external::op_mul_int(({let db = (var_self); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((0_i32)))} }), ({let db = (var_self); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((0_i32)))} }))), (external::op_mul_int(({let db = (var_self); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} }), ({let db = (var_self); if db.rec == 0 { i32::MIN } else { stores.store(&db).get_int(db.rec, db.pos + u32::from((4_i32)))} }))))
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    let _pre13 = { //Object_3: ref(Pt)["__ref_1"]
      OpDatabase(stores, var___ref_1, 18_i32);
      {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (3_i32));};
      {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (4_i32));};
      var___ref_1
      } /*Object_3: ref(Pt)["__ref_1"]*/;
    t_2Pt_dist2(stores, _pre13)
    } /*block_2: integer*/;
  let _pre14 = { //Formatted string_4: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 25";
    &var___work_1
    } /*Formatted string_4: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (25_i32), _pre14, "method_on_constructor", 7_i32);
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_method_on_constructor() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
