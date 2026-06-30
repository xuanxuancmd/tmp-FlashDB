// tests/e2e_coexistence.rs — E2E: KVDB + TSDB coexistence on separate flash devices
//
// Verifies that KVDB and TSDB can coexist in the same binary, each using
// their own FlashDevice instance, without interference.

use flashdb::{
    blob_make, FdbDefaultKv, FdbKvdb, FdbTsdb, FlashDevice, MockFlash,
};

fn get_time_zero() -> flashdb::FdbTime {
    0
}

#[test]
fn test_kvdb_tsdb_coexistence() {
    // --- KVDB on its own flash ---
    let mut kv_flash = MockFlash::new("fdb_kvdb1", 4096, 16384, 4096);
    let mut kvdb = FdbKvdb::default();
    kvdb.set_sec_size(4096);
    kvdb.parent.max_size = 16384;
    let result = kvdb.kvdb_init(
        &mut kv_flash,
        "config",
        "fdb_kvdb1",
        FdbDefaultKv { kvs: &[] },
    );
    assert!(result.is_ok(), "KVDB init failed: {:?}", result);

    // Write a KV
    let result = kvdb.kv_set(&mut kv_flash, "hostname", "sensor-01");
    assert!(result.is_ok(), "kv_set failed: {:?}", result);

    // Read it back
    let value = kvdb.kv_get(&mut kv_flash, "hostname");
    assert_eq!(value, Some("sensor-01".to_string()));

    // --- TSDB on its own flash ---
    let mut ts_flash = MockFlash::new("fdb_tsdb1", 4096, 16384, 4096);
    let mut tsdb = FdbTsdb::default();
    tsdb.set_sec_size(4096);
    tsdb.parent.max_size = 16384;
    let result = tsdb.init(
        &mut ts_flash,
        "logdb",
        "fdb_tsdb1",
        get_time_zero,
        256,
    );
    assert!(result.is_ok(), "TSDB init failed: {:?}", result);

    // Append a TSL
    let mut buf = [0u8; 64];
    let blob = blob_make(&mut buf);
    let result = tsdb.tsl_append_with_ts(&mut ts_flash, &blob, 100);
    assert!(result.is_ok(), "tsl_append failed: {:?}", result);
    assert_eq!(tsdb.last_time(), 100);

    // --- Verify no cross-interference ---
    // KVDB data should be unchanged
    let value = kvdb.kv_get(&mut kv_flash, "hostname");
    assert_eq!(
        value,
        Some("sensor-01".to_string()),
        "KVDB data corrupted by TSDB operations"
    );

    // TSDB data should be intact
    let mut count = 0usize;
    tsdb.tsl_iter(&ts_flash, |tsl| {
        if tsl.status == flashdb::FdbTslStatus::Write {
            count += 1;
        }
        false
    });
    assert_eq!(count, 1, "TSDB should have 1 TSL after KVDB operations");
}

#[test]
fn test_kvdb_tsdb_separate_state() {
    // Verify that KVDB and TSDB have completely independent state.
    let mut kv_flash = MockFlash::new("fdb_kvdb1", 4096, 16384, 4096);
    let mut ts_flash = MockFlash::new("fdb_tsdb1", 4096, 16384, 4096);

    let mut kvdb = FdbKvdb::default();
    kvdb.set_sec_size(4096);
    kvdb.parent.max_size = 16384;
    kvdb.kvdb_init(
        &mut kv_flash,
        "config",
        "fdb_kvdb1",
        FdbDefaultKv { kvs: &[] },
    )
    .expect("KVDB init");

    let mut tsdb = FdbTsdb::default();
    tsdb.set_sec_size(4096);
    tsdb.parent.max_size = 16384;
    tsdb.init(
        &mut ts_flash,
        "logdb",
        "fdb_tsdb1",
        get_time_zero,
        128,
    )
    .expect("TSDB init");

    // Verify init_ok flags are independent
    assert!(kvdb.parent.init_ok, "KVDB should be initialised");
    assert!(tsdb.parent.init_ok, "TSDB should be initialised");

    // Deinit KVDB should not affect TSDB
    kvdb.kvdb_deinit().expect("KVDB deinit");
    assert!(!kvdb.parent.init_ok, "KVDB should be deinitialised");
    assert!(
        tsdb.parent.init_ok,
        "TSDB should still be initialised after KVDB deinit"
    );

    // TSDB operations should still work
    let mut buf = [0u8; 32];
    let blob = blob_make(&mut buf);
    let result = tsdb.tsl_append_with_ts(&mut ts_flash, &blob, 200);
    assert!(result.is_ok(), "TSDB append should work after KVDB deinit");
    assert_eq!(tsdb.last_time(), 200);
}
