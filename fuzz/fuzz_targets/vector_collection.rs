// @PLAN53 Wave 2 — structure-aware fuzzing of the inline by-value vector.
//
// A loft vector is reached through a *handle field*: `db` points at a `u32`
// slot holding the vector record's offset; the record itself is
// `[claim:i32][length:u32][elements…]`.  This harness drives random
// append/get/remove sequences and checks the loft vector against a `Vec<i64>`
// reference model after every op (length parity + element-by-element
// read-back), over ASan + the store's own debug validators.
#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use loft::keys::DbRef;
use loft::store::Store;
use loft::vector::{get_vector, length_vector, remove_vector, vector_append, vector_finish};

const ELEM: u32 = 8; // i64 elements

#[derive(Arbitrary, Debug)]
enum Op {
    Append(i64),
    Get(u16),
    Remove(u16),
}

fuzz_target!(|ops: Vec<Op>| {
    // One store, and a 2-word "handle" record holding the vector pointer at
    // byte 8 (past the [claim][length] header area) — initialised to 0 (no
    // vector yet).  `db` is the handle.
    let mut stores = vec![Store::new_in_use(64)];
    let handle_rec = stores[0].claim(2);
    stores[0].set_u32_raw(handle_rec, 8, 0);
    let db = DbRef {
        store_nr: 0,
        rec: handle_rec,
        pos: 8,
    };

    let mut model: Vec<i64> = Vec::new();

    for op in ops {
        match op {
            Op::Append(v) => {
                let slot = vector_append(&db, ELEM, &mut stores);
                // slot.rec == 0 only if db.rec == 0, which never happens here.
                assert!(slot.rec != 0, "vector_append returned a null slot");
                stores[slot.store_nr as usize].set_int(slot.rec, slot.pos, v);
                vector_finish(&db, &mut stores);
                model.push(v);
            }
            Op::Get(raw) => {
                let idx = usize::from(raw) % (model.len() + 2); // 0..=len+1 (incl. OOR)
                let r = get_vector(&db, ELEM, idx as i64, &stores);
                if idx < model.len() {
                    assert!(r.rec != 0, "get({idx}) null for a live element");
                    assert_eq!(
                        stores[r.store_nr as usize].get_int(r.rec, r.pos),
                        model[idx],
                        "get({idx}) value mismatch",
                    );
                } else {
                    assert!(
                        r.rec == 0,
                        "get({idx}) returned non-null past end (len={})",
                        model.len()
                    );
                }
            }
            Op::Remove(raw) => {
                let idx = usize::from(raw) % (model.len() + 2);
                let ok = remove_vector(&db, ELEM, idx as i64, &mut stores);
                if idx < model.len() {
                    assert!(ok, "remove({idx}) failed on a live element");
                    model.remove(idx);
                } else {
                    assert!(
                        !ok,
                        "remove({idx}) succeeded past end (len={})",
                        model.len()
                    );
                }
            }
        }

        // Invariants after every op: length parity + full read-back.
        assert_eq!(
            length_vector(&db, &stores),
            model.len() as u32,
            "length divergence",
        );
        for (i, &expected) in model.iter().enumerate() {
            let r = get_vector(&db, ELEM, i as i64, &stores);
            assert!(r.rec != 0, "element {i} vanished (len={})", model.len());
            assert_eq!(
                stores[r.store_nr as usize].get_int(r.rec, r.pos),
                expected,
                "element {i} corrupted",
            );
        }
    }
});
