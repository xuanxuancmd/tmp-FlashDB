// c-port: kvdb_equiv.rs — KVDB C-equivalence integration test
//
// Behavioural equivalence checks of the Rust KVDB against the golden semantics
// of the C FlashDB (fdb_kvdb.c). These tests exercise the public API end-to-end
// through a NOR-flash simulation, mirroring the scenarios the C author verified:
//   * init on erased flash (auto-format / set_default)
//   * CRUD round-trip (set → get → update → del)
//   * binary blob set/get
//   * GC preserving live KVs across many updates
//   * iterator visiting all live KVs in sector order
//   * set_default wiping custom KVs and restoring defaults
//   * integrity check on a healthy database
//
// `MockFlash` is `#[cfg(test)]`-gated inside the library and therefore invisible
// to integration tests (a separate crate), so this file provides its own NOR-flash
// simulation implementing `FlashDevice` (identical to foundation_equiv.rs).

use flashdb::def::FDB_BYTE_ERASED;
use flashdb::{
    blob_make, blob_read, FdbBlob, FdbDefaultKv, FdbDefaultKvNode, FdbErr, FdbKv, FdbKvdb,
    FlashDevice,
};

/// Minimal in-memory NOR flash for the equivalence test.
/// Initial state all 0xFF; write ANDs (1->0 only); erase resets to 0xFF.
struct EquivFlash {
    data: Vec<u8>,
}

impl EquivFlash {
    fn new(size: usize) -> Self {
        Self {
            data: vec![FDB_BYTE_ERASED; size],
        }
    }
}

impl FlashDevice for EquivFlash {
    fn read(&self, addr: u32, buf: &mut [u8]) -> Result<(), FdbErr> {
        let addr = addr as usize;
        let end = addr.checked_add(buf.len()).ok_or(FdbErr::ReadErr)?;
        if end > self.data.len() {
            return Err(FdbErr::ReadErr);
        }
        buf.copy_from_slice(&self.data[addr..end]);
        Ok(())
    }
    fn write(&mut self, addr: u32, buf: &[u8]) -> Result<(), FdbErr> {
        let addr = addr as usize;
        let end = addr.checked_add(buf.len()).ok_or(FdbErr::WriteErr)?;
        if end > self.data.len() {
            return Err(FdbErr::WriteErr);
        }
        for (i, &b) in buf.iter().enumerate() {
            self.data[addr + i] &= b; // NOR: can only change 1->0
        }
        Ok(())
    }
    fn erase(&mut self, addr: u32, size: u32) -> Result<(), FdbErr> {
        let addr = addr as usize;
        let end = addr.checked_add(size as usize).ok_or(FdbErr::EraseErr)?;
        if end > self.data.len() {
            return Err(FdbErr::EraseErr);
        }
        self.data[addr..end].fill(FDB_BYTE_ERASED);
        Ok(())
    }
    fn len(&self) -> usize {
        self.data.len()
    }
}

/// Build a configured KVDB + erased NOR flash (4 sectors of 4096 bytes).
fn make_kvdb() -> (FdbKvdb, EquivFlash) {
    let mut db = FdbKvdb::default();
    db.parent.sec_size = 4096;
    db.parent.max_size = 16384;
    let flash = EquivFlash::new(16384);
    (db, flash)
}

// ---------------------------------------------------------------------------
// init + CRUD round-trip (c: fdb_kvdb_init / fdb_kv_set / fdb_kv_get / fdb_kv_del)
// ---------------------------------------------------------------------------

#[test]
fn c_equiv_init_and_crud() {
    let (mut db, mut flash) = make_kvdb();
    db.kvdb_init(&mut flash, "kv", "part", FdbDefaultKv { kvs: &[] })
        .unwrap();
    assert!(db.parent.init_ok, "init must set init_ok");

    // set → get
    db.kv_set(&mut flash, "key1", "value1").unwrap();
    assert_eq!(db.kv_get(&mut flash, "key1"), Some("value1".to_string()));

    // update → get (latest value wins)
    db.kv_set(&mut flash, "key1", "value2").unwrap();
    assert_eq!(db.kv_get(&mut flash, "key1"), Some("value2".to_string()));

    // del → get returns None
    db.kv_del(&mut flash, "key1").unwrap();
    assert_eq!(db.kv_get(&mut flash, "key1"), None);

    // del absent key → KvNameErr
    assert_eq!(db.kv_del(&mut flash, "nope"), Err(FdbErr::KvNameErr));
}

#[test]
fn c_equiv_multiple_keys() {
    let (mut db, mut flash) = make_kvdb();
    db.kvdb_init(&mut flash, "kv", "part", FdbDefaultKv { kvs: &[] })
        .unwrap();
    db.kv_set(&mut flash, "alpha", "1").unwrap();
    db.kv_set(&mut flash, "beta", "2").unwrap();
    db.kv_set(&mut flash, "gamma", "3").unwrap();
    assert_eq!(db.kv_get(&mut flash, "alpha"), Some("1".to_string()));
    assert_eq!(db.kv_get(&mut flash, "beta"), Some("2".to_string()));
    assert_eq!(db.kv_get(&mut flash, "gamma"), Some("3".to_string()));

    // delete the middle key; others unaffected
    db.kv_del(&mut flash, "beta").unwrap();
    assert_eq!(db.kv_get(&mut flash, "alpha"), Some("1".to_string()));
    assert_eq!(db.kv_get(&mut flash, "beta"), None);
    assert_eq!(db.kv_get(&mut flash, "gamma"), Some("3".to_string()));
}

// ---------------------------------------------------------------------------
// binary blob set/get (c: fdb_kv_set_blob / fdb_kv_get_blob)
// ---------------------------------------------------------------------------

#[test]
fn c_equiv_blob_round_trip() {
    let (mut db, mut flash) = make_kvdb();
    db.kvdb_init(&mut flash, "kv", "part", FdbDefaultKv { kvs: &[] })
        .unwrap();

    let data: Vec<u8> = (0x01..=0x20).collect();
    let mut write_buf = data.clone();
    let mut blob = blob_make(&mut write_buf);
    db.kv_set_blob(&mut flash, "bin", &mut blob).unwrap();

    let mut read_buf = vec![0u8; 32];
    let mut blob2 = blob_make(&mut read_buf);
    let read = db.kv_get_blob(&mut flash, "bin", &mut blob2);
    assert_eq!(read, Some(32));
    assert_eq!(&read_buf[..32], &data[..], "blob must round-trip exactly");
}

#[test]
fn c_equiv_blob_via_kv_to_blob() {
    // c: fdb_kv_get_obj + fdb_kv_to_blob + fdb_blob_read
    let (mut db, mut flash) = make_kvdb();
    db.kvdb_init(&mut flash, "kv", "part", FdbDefaultKv { kvs: &[] })
        .unwrap();
    db.kv_set(&mut flash, "k", "hello").unwrap();

    let mut kv = FdbKv::default();
    assert!(db.kv_get_obj(&mut flash, "k", &mut kv));

    let mut buf = [0u8; 16];
    let mut blob: FdbBlob = blob_make(&mut buf);
    flashdb::kvdb::kv_to_blob(&kv, &mut blob);
    let n = blob_read(&flash, &mut blob);
    assert_eq!(n, 5);
    assert_eq!(&buf[..5], b"hello");
}

// ---------------------------------------------------------------------------
// GC preserves live KVs across many updates (c: gc_collect triggered by set_kv)
// ---------------------------------------------------------------------------

#[test]
fn c_equiv_gc_preserves_live_kv() {
    let (mut db, mut flash) = make_kvdb();
    db.kvdb_init(&mut flash, "kv", "part", FdbDefaultKv { kvs: &[] })
        .unwrap();

    // Repeatedly update the same key; old versions are deleted, sectors fill up,
    // and GC must reclaim space while keeping the latest value intact.
    for i in 0..200 {
        let val = format!("value_{i}");
        db.kv_set(&mut flash, "counter", &val).unwrap();
    }
    assert_eq!(
        db.kv_get(&mut flash, "counter"),
        Some("value_199".to_string()),
        "GC must preserve the latest value of an updated key"
    );

    // a separate key written before the updates must also survive
    db.kv_set(&mut flash, "stable", "keep").unwrap();
    for i in 0..200 {
        let val = format!("v{i}");
        db.kv_set(&mut flash, "counter", &val).unwrap();
    }
    assert_eq!(db.kv_get(&mut flash, "stable"), Some("keep".to_string()));
    assert_eq!(db.kv_get(&mut flash, "counter"), Some("v199".to_string()));
}

// ---------------------------------------------------------------------------
// iterator visits all live KVs (c: fdb_kv_iterator_init / fdb_kv_iterate)
// ---------------------------------------------------------------------------

#[test]
fn c_equiv_iterator_visits_all() {
    let (mut db, mut flash) = make_kvdb();
    db.kvdb_init(&mut flash, "kv", "part", FdbDefaultKv { kvs: &[] })
        .unwrap();
    db.kv_set(&mut flash, "a", "1").unwrap();
    db.kv_set(&mut flash, "b", "2").unwrap();
    db.kv_set(&mut flash, "c", "3").unwrap();

    let mut itr = db.kv_iterator_init();
    let mut keys: Vec<String> = Vec::new();
    while db.kv_iterate(&mut flash, &mut itr) {
        keys.push(itr.curr_kv.name_str().to_string());
    }
    assert_eq!(keys.len(), 3, "iterator must visit exactly the 3 live KVs");
    assert!(keys.contains(&"a".to_string()));
    assert!(keys.contains(&"b".to_string()));
    assert!(keys.contains(&"c".to_string()));

    // after deleting one, the iterator visits the remaining two
    db.kv_del(&mut flash, "b").unwrap();
    let mut itr2 = db.kv_iterator_init();
    let mut count = 0;
    while db.kv_iterate(&mut flash, &mut itr2) {
        count += 1;
    }
    assert_eq!(count, 2, "deleted KV must not appear in iteration");
}

// ---------------------------------------------------------------------------
// set_default wipes custom KVs and restores defaults (c: fdb_kv_set_default)
// ---------------------------------------------------------------------------

#[test]
fn c_equiv_set_default() {
    static DEFAULT_KVS: &[FdbDefaultKvNode] = &[
        FdbDefaultKvNode {
            key: "boot_count",
            value: b"0",
            value_len: 0,
        },
        FdbDefaultKvNode {
            key: "version",
            value: b"1.0",
            value_len: 0,
        },
    ];
    let (mut db, mut flash) = make_kvdb();
    db.kvdb_init(
        &mut flash,
        "kv",
        "part",
        FdbDefaultKv { kvs: DEFAULT_KVS },
    )
    .unwrap();
    // defaults present after init (init on empty flash → set_default)
    assert_eq!(db.kv_get(&mut flash, "boot_count"), Some("0".to_string()));
    assert_eq!(db.kv_get(&mut flash, "version"), Some("1.0".to_string()));

    // write a custom KV
    db.kv_set(&mut flash, "custom", "x").unwrap();
    assert_eq!(db.kv_get(&mut flash, "custom"), Some("x".to_string()));

    // set_default wipes custom, restores defaults
    db.kv_set_default(&mut flash).unwrap();
    assert_eq!(db.kv_get(&mut flash, "custom"), None);
    assert_eq!(db.kv_get(&mut flash, "boot_count"), Some("0".to_string()));
    assert_eq!(db.kv_get(&mut flash, "version"), Some("1.0".to_string()));
}

// ---------------------------------------------------------------------------
// integrity check (c: fdb_kvdb_check)
// ---------------------------------------------------------------------------

#[test]
fn c_equiv_check_healthy() {
    let (mut db, mut flash) = make_kvdb();
    db.kvdb_init(&mut flash, "kv", "part", FdbDefaultKv { kvs: &[] })
        .unwrap();
    db.kv_set(&mut flash, "k1", "v1").unwrap();
    db.kv_set(&mut flash, "k2", "v2").unwrap();
    db.kv_set(&mut flash, "k3", "v3").unwrap();
    let result = db.kvdb_check(&mut flash);
    assert!(result.is_ok(), "check must pass on a healthy db");

    // deinit then operations are rejected
    db.kvdb_deinit().unwrap();
    assert_eq!(db.kv_get(&mut flash, "k1"), None, "get after deinit returns None");
}

// ---------------------------------------------------------------------------
// re-init preserves persisted data (c: recovery on reboot)
// ---------------------------------------------------------------------------

#[test]
fn c_equiv_reinit_preserves_data() {
    let (mut db, mut flash) = make_kvdb();
    db.kvdb_init(&mut flash, "kv", "part", FdbDefaultKv { kvs: &[] })
        .unwrap();
    db.kv_set(&mut flash, "persisted", "yes").unwrap();
    db.kv_set(&mut flash, "count", "42").unwrap();

    // simulate a reboot: drop the db object, re-init on the same flash
    let mut db2 = FdbKvdb::default();
    db2.parent.sec_size = 4096;
    db2.parent.max_size = 16384;
    db2.kvdb_init(&mut flash, "kv", "part", FdbDefaultKv { kvs: &[] })
        .unwrap();
    assert_eq!(db2.kv_get(&mut flash, "persisted"), Some("yes".to_string()));
    assert_eq!(db2.kv_get(&mut flash, "count"), Some("42".to_string()));
}
