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
    db.field(s, "name", 6);
    db.field(s, "number", 0);
    let s = db.structure("main_vector<Data>", 0); // 19
    let vec_vector = db.vector(18);
    db.field(s, "vector", vec_vector);
    db.vector(18);
    db.finish();
}

fn n_data(stores: &mut Stores, mut var_n: Str, mut var_res: DbRef) -> DbRef { //block_1: vector<ref(Data)>["res"]
  vector::clear_vector(&(var_res), &mut s.database.allocations);;
  let mut var_nr: i32 = 0_i32;
  { //For block_2: void
    let mut var_ch__index: i32 = 0_i32;
    let mut var_ch__next: i32 = 0_i32;
    loop { //For loop_3
      let mut var_ch: i32 = { //for text next_4: character
        var_ch__index = var_ch__next;
        let mut var__for_result_1: i32 = external::text_character((&var_n), (var_ch__next));
        var_ch__next = external::op_add_int((var_ch__next), (OpLengthCharacter(stores, var__for_result_1)));
        var__for_result_1
        } /*for text next_4: character*/;
      if !(external::op_conv_bool_from_character((var_ch))) { //break_5: void
        break;
        } /*break_5: void*/ else {()};
      { //block_6: void
        let mut var__elm_2: DbRef = OpNewRecord(stores, var_res, 20_i32, 65535_i32);
        {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (var_ch as u32 as i32));};
        {let db = (var__elm_2); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((4_i32)), (var_nr));};
        OpFinishRecord(stores, var_res, var__elm_2, 20_i32, 65535_i32);
        var_nr = external::op_add_int((var_nr), (1_i32));
        } /*block_6: void*/;
      } /*For loop_3*/;
    } /*For block_2: void*/;
  var_res
  } /*block_1: vector<ref(Data)>["res"]*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___ref_1: DbRef = stores.null();
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    let mut var_d: DbRef = n_data(stores, "test", var___ref_1);
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_d, 20_i32, 0_i32);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"[{name:'t',number:0},{name:'e',number:1},{name:'s',number:2},{name:'t',number:3}]\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("[{name:'t',number:0},{name:'e',number:1},{name:'s',number:2},{name:'t',number:3}]"), _pre13, "fill_fn", 19_i32);
  ;
  ;
  ;
  OpFreeRef(stores, var___ref_1);
  } /*block_1: void*/

#[test]
fn code_fill_fn() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
