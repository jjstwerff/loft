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
    db.field(s, "a", 0);
    db.field(s, "bb", 5);
    db.field(s, "ccc", 4);
    db.finish();
}

fn n_obj(stores: &mut Stores) -> DbRef { //block_1: ref(Object)
  let mut var___ref_1: DbRef = stores.null();
  { //Object_2: ref(Object)["__ref_1"]
    OpDatabase(stores, var___ref_1, 18_i32);
    {let db = (var___ref_1); stores.store_mut(&db).set_int(db.rec, db.pos + u32::from((0_i32)), (12_i32));};
    {let db = (var___ref_1); let s_val = ("hi").to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    {let db = (var___ref_1); stores.store_mut(&db).set_byte(db.rec, db.pos + u32::from((8_i32)), i32::from((0_i32)), (if false {1_i32} else {0_i32}));};
    var___ref_1
    } /*Object_2: ref(Object)["__ref_1"]*/
  } /*block_1: ref(Object)*/

fn n_test(stores: &mut Stores) { //block_1: void
  let mut var___work_2: String = "".to_string();
  let mut var___work_1: String = "".to_string();
  let mut var_test_value = { //block_2: text["__work_1"]
    let mut var_o: DbRef = n_obj(stores);
    let mut var__field_1: String = {let db = (var_o); let store = stores.store(&db); Str::new(store.get_str(store.get_int(db.rec, db.pos + u32::from((4_i32))) as u32))}.to_string();
    {let c = 33_i32; if c != 0 { var__field_1.push(ops::to_char(c)); } };
    {let db = (var_o); let s_val = (&var__field_1).to_string(); let store = stores.store_mut(&db); let s_pos = store.set_str(&s_val); store.set_int(db.rec, db.pos + u32::from((4_i32)), s_pos as i32);};
    { //Formatted string_3: text["__work_1"]
      var___work_1 = "".to_string();
      OpFormatDatabase(stores, &var___work_1, var_o, 18_i32, 0_i32);
      var___work_1 += " pretty ";
      OpFormatDatabase(stores, &var___work_1, var_o, 18_i32, 1_i32);
      ;
      OpFreeRef(stores, var_o);
      &var___work_1
      } /*Formatted string_3: text["__work_1"]*/
    } /*block_2: text["__work_1"]*/.to_string();
  let _pre13 = { //Formatted string_4: text["__work_2"]
    var___work_2 = "Test failed ".to_string();
    ops::format_text(&mut var___work_2, &var_test_value, 0_i32, -1, 32);
    var___work_2 += " != \"{a:12,bb:\"hi!\",ccc:false} pretty { a: 12, bb: \"hi!\", ccc: false }\"";
    &var___work_2
    } /*Formatted string_4: text["__work_2"]*/;
  n_assert(stores, (&var_test_value) == ("{a:12,bb:\"hi!\",ccc:false} pretty { a: 12, bb: \"hi!\", ccc: false }"), _pre13, "print_object", 7_i32);
  ;
  ;
  ;
  } /*block_1: void*/

#[test]
fn code_print_object() {
    let mut stores = Stores::new();
    init(&mut stores);
    n_test(&mut stores);
}
