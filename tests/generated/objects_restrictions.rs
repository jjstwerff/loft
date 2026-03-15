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
    let byte_byte = db.byte(0, false);
    db.field(s, "byte", byte_byte);
    let byte_val = db.byte(1, true);
    db.field(s, "val", byte_val);
    let byte_signed = db.byte(-127, true);
    db.field(s, "signed", byte_signed);
    db.finish();
}

fn t_4Data_calc(stores: &mut Stores, mut var_self: DbRef) -> i32 { //block_1: integer
  external::op_add_int((external::op_add_int((external::op_mul_int(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((1_i32)), i32::from((1_i32)))}), (65536_i32))), (external::op_mul_int(({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((0_i32)), i32::from((0_i32)))}), (256_i32))))), ({let db = (var_self); stores.store(&db).get_byte(db.rec, db.pos + u32::from((2_i32)), i32::from((-127_i32)))}))
  } /*block_1: integer*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_1: String = "".to_string();
  let mut var_test_value: i32 = { //block_2: integer
    1_i32
    } /*block_2: integer*/;
  let _pre13 = { //Formatted string_3: text["__work_1"]
    var___work_1 = "Test failed ".to_string();
    ops::format_long(&mut var___work_1, external::op_conv_long_from_int((var_test_value)), 10 as u8, 0_i32, 32 as u8, false, false);
    var___work_1 += " != 1";
    &var___work_1
    } /*Formatted string_3: text["__work_1"]*/;
  n_assert(stores, (var_test_value) == (1_i32), _pre13, "restrictions", 15_i32);
  ;
  } /*block_1: void*/

#[test]
fn code_restrictions() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
