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
    let s = db.structure("main_vector<float>", 0); // 18
    let vec_vector = db.vector(3);
    db.field(s, "vector", vec_vector);
    db.vector(3);
    db.finish();
}

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: f64 = { //block_2: float
    let mut var_a: DbRef = s.db_from_text(("[ 1.2, -10.3, 1.812e4, 1.001e-8 ]"), (19_i32));
    let _ret = ({let db = (vector::get_vector(&(var_a), u32::from((8_i32)), (2_i32), &s.database.allocations)); stores.store(&db).get_float(db.rec, db.pos + u32::from((0_i32)))}) + ({let db = (vector::get_vector(&(var_a), u32::from((8_i32)), (3_i32), &s.database.allocations)); stores.store(&db).get_float(db.rec, db.pos + u32::from((0_i32)))});
    OpFreeRef(stores, var_a);
    _ret
    } /*block_2: float*/;
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    OpFormatFloat(stores, &var___work_1, var_test_value, 0_i32, 0_i32);
    var___work_1 += " != 18120.00000001001";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, ((var_test_value) - (18120.00000001001_f64)).abs() < 0.000_000_001f64, _pre13, "parse_vector", 4_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_parse_vector() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
