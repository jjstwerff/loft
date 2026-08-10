// #306 regression — the store-slot space is u16 and store_nr 65535 is the
// null-DbRef sentinel.  Before the fix, allocating the 65536th live store
// wrapped the `max` watermark to 0 and the allocator handed out slot 0 (the
// interpreter's eval-stack store) as a fresh store, silently corrupting the
// whole runtime.  The allocator must now refuse loudly instead.
use loft::database::Stores;

// @speed 0.6
#[test]
#[should_panic(expected = "store table exhausted")]
fn slot_exhaustion_is_loud() {
    let mut stores = Stores::new();
    for _ in 0..66_000u32 {
        let _ = stores.database(16);
    }
}
