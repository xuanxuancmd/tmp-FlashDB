// c-port: tsdb_equiv.rs — C-equivalence integration test for TSDB
//
// Migrates the golden test patterns from the C project's
// `tests/fdb_tsdb_tc.c` to verify that the Rust TSDB implementation behaves
// identically to the C version. The test data (append order, iter
// forward/reverse, range query boundaries, query_count, set_status, clean,
// reboot persistence) are the C author's verified reference behaviour and
// exist to catch translation drift.
//
// The library's own `MockFlash` is `#[cfg(test)]`-gated and therefore invisible
// to integration tests (a separate crate), so this file provides its own
// minimal NOR-flash simulation implementing `FlashDevice` (same pattern as
// `tests/foundation_equiv.rs`).

use flashdb::def::{FdbErr, FdbGetTime, FdbTime, FdbTsl, FdbTslStatus, FdbTsdb, FDB_BYTE_ERASED};
use flashdb::{blob_make, blob_read, FlashDevice};

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
        for (i, &byte) in buf.iter().enumerate() {
            self.data[addr + i] &= byte;
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

// ===== Test configuration (mirrors fdb_tsdb_tc.c) =====

const SECTOR_SIZE: u32 = 4096;
const DB_SIZE: u32 = SECTOR_SIZE * 16; // 16 sectors
const MAX_LEN: usize = 128;
const TIME_STEP: FdbTime = 2;
const TSL_COUNT: usize = 256;
const USER_STATUS1_COUNT: usize = TSL_COUNT / 2;
const DELETED_COUNT: usize = TSL_COUNT - USER_STATUS1_COUNT;

/// Simple get_time function (returns a fixed value; we use append_with_ts
/// for explicit timestamps).
fn test_get_time() -> FdbTime {
    0
}

/// Initialize a TSDB backed by EquivFlash.
fn make_tsdb(flash: &mut EquivFlash) -> FdbTsdb {
    let mut db = FdbTsdb::default();
    db.set_sec_size(SECTOR_SIZE);
    db.set_max_size(DB_SIZE);
    db.init(flash, "test_ts", "test_part", test_get_time as FdbGetTime, MAX_LEN)
        .expect("init should succeed");
    db
}

/// Append `count` TSLs with timestamps TIME_STEP, 2*TIME_STEP, ..., count*TIME_STEP.
/// Blob data is the timestamp as a little-endian i32.
fn append_tsls(db: &mut FdbTsdb, flash: &mut EquivFlash, count: usize) {
    for i in 1..=count {
        let ts = i as FdbTime * TIME_STEP;
        let mut data = ts.to_le_bytes();
        let mut blob = blob_make(&mut data);
        blob.size = 4;
        db.tsl_append_with_ts(flash, &blob, ts)
            .expect("append should succeed");
    }
}

// ===== Test cases (migrated from fdb_tsdb_tc.c) =====

/// c: fdb_tsdb_tc.c:116-126 — test_fdb_tsl_append
/// c: fdb_tsdb_tc.c:148-152 — test_fdb_tsl_iter
#[test]
fn test_c_equiv_append_and_iter() {
    let mut flash = EquivFlash::new(DB_SIZE as usize);
    let mut db = make_tsdb(&mut flash);
    append_tsls(&mut db, &mut flash, TSL_COUNT);

    // Iterate forward and verify each TSL's time matches its blob data
    let mut idx = 0;
    db.tsl_iter(&flash, |tsl| {
        idx += 1;
        let expected_time = idx as FdbTime * TIME_STEP;
        assert_eq!(tsl.time, expected_time, "TSL {} time mismatch", idx);
        assert_eq!(tsl.status, FdbTslStatus::Write, "TSL {} must be WRITE", idx);
        assert_eq!(tsl.log_len, 4, "TSL {} log_len must be 4", idx);

        // Read blob data and verify it matches the timestamp
        let mut read_buf = [0u8; 4];
        let mut read_blob = blob_make(&mut read_buf);
        db.tsl_to_blob(tsl, &mut read_blob);
        let read_len = blob_read(&flash, &mut read_blob);
        assert_eq!(read_len, 4);
        let read_time = FdbTime::from_le_bytes(read_buf);
        assert_eq!(read_time, expected_time, "TSL {} blob data mismatch", idx);

        false // continue
    });
    assert_eq!(idx, TSL_COUNT, "iter must visit all {} TSLs", TSL_COUNT);
}

/// c: fdb_tsdb_tc.c:110-114 — fdb_reboot (deinit + init)
#[test]
fn test_c_equiv_reboot_persistence() {
    let mut flash = EquivFlash::new(DB_SIZE as usize);

    // Boot 1: init + append
    {
        let mut db = make_tsdb(&mut flash);
        append_tsls(&mut db, &mut flash, TSL_COUNT);
        assert_eq!(db.last_time(), TSL_COUNT as FdbTime * TIME_STEP);
        db.deinit().unwrap();
    }

    // Boot 2: re-init + verify
    {
        let db = make_tsdb(&mut flash);
        assert_eq!(
            db.last_time(),
            TSL_COUNT as FdbTime * TIME_STEP,
            "last_time must persist across reboot"
        );

        let mut count = 0;
        db.tsl_iter(&flash, |_tsl| {
            count += 1;
            false
        });
        assert_eq!(count, TSL_COUNT, "all TSLs must persist across reboot");
    }
}

/// c: fdb_tsdb_tc.c:154-163 — test_fdb_tsl_iter_by_time
#[test]
fn test_c_equiv_iter_by_time() {
    let mut flash = EquivFlash::new(DB_SIZE as usize);
    let mut db = make_tsdb(&mut flash);
    append_tsls(&mut db, &mut flash, TSL_COUNT);

    // For each timestamp, iter_by_time(ts, ts) must return exactly 1 TSL
    for i in 1..=TSL_COUNT {
        let cur = i as FdbTime * TIME_STEP;
        let mut count = 0;
        db.tsl_iter_by_time(&flash, cur, cur, |tsl| {
            assert_eq!(tsl.time, cur, "single-timestamp query must match");
            count += 1;
            false
        });
        assert_eq!(count, 1, "iter_by_time({},{}) must return 1 TSL", cur, cur);
    }

    // Full range [0, TSL_COUNT*TIME_STEP] must return all TSLs
    let mut count = 0;
    db.tsl_iter_by_time(&flash, 0, TSL_COUNT as FdbTime * TIME_STEP, |_tsl| {
        count += 1;
        false
    });
    assert_eq!(count, TSL_COUNT, "full range must return all TSLs");

    // Reverse range (from > to)
    let mut count = 0;
    let mut last_time = FdbTime::MAX;
    db.tsl_iter_by_time(
        &flash,
        TSL_COUNT as FdbTime * TIME_STEP,
        TIME_STEP,
        |tsl| {
            assert!(tsl.time <= last_time, "reverse iter must be descending");
            last_time = tsl.time;
            count += 1;
            false
        },
    );
    assert_eq!(count, TSL_COUNT, "reverse full range must return all TSLs");
}

/// c: fdb_tsdb_tc.c:165-176 — test_fdb_tsl_query_count
#[test]
fn test_c_equiv_query_count() {
    let mut flash = EquivFlash::new(DB_SIZE as usize);
    let mut db = make_tsdb(&mut flash);
    append_tsls(&mut db, &mut flash, TSL_COUNT);

    let count = db.tsl_query_count(
        &flash,
        0,
        TSL_COUNT as FdbTime * TIME_STEP,
        FdbTslStatus::Write,
    );
    assert_eq!(count, TSL_COUNT, "query_count WRITE must be {}", TSL_COUNT);

    let count = db.tsl_query_count(&flash, 0, 0, FdbTslStatus::Write);
    assert_eq!(count, 0, "query_count [0,0] must be 0");

    // Partial range
    let half = TSL_COUNT / 2;
    let count = db.tsl_query_count(
        &flash,
        TIME_STEP,
        half as FdbTime * TIME_STEP,
        FdbTslStatus::Write,
    );
    assert_eq!(count, half, "query_count [2, {}] must be {}", half * 2, half);
}

/// c: fdb_tsdb_tc.c:191-200 — test_fdb_tsl_set_status
#[test]
fn test_c_equiv_set_status() {
    let mut flash = EquivFlash::new(DB_SIZE as usize);
    let mut db = make_tsdb(&mut flash);
    append_tsls(&mut db, &mut flash, TSL_COUNT);

    let from = 0;
    let to = TSL_COUNT as FdbTime * TIME_STEP;

    // Set first half to USER_STATUS1, second half to DELETED
    let boundary = USER_STATUS1_COUNT as FdbTime * TIME_STEP;
    let mut tsls_to_update: Vec<FdbTsl> = Vec::new();
    db.tsl_iter(&flash, |tsl| {
        tsls_to_update.push(*tsl);
        false
    });

    for tsl in &tsls_to_update {
        let status = if tsl.time <= boundary {
            FdbTslStatus::UserStatus1
        } else {
            FdbTslStatus::Deleted
        };
        db.tsl_set_status(&mut flash, tsl, status).unwrap();
    }

    // Verify counts
    let user_count = db.tsl_query_count(&flash, from, to, FdbTslStatus::UserStatus1);
    assert_eq!(
        user_count, USER_STATUS1_COUNT,
        "USER_STATUS1 count must be {}",
        USER_STATUS1_COUNT
    );

    let deleted_count = db.tsl_query_count(&flash, from, to, FdbTslStatus::Deleted);
    assert_eq!(
        deleted_count, DELETED_COUNT,
        "DELETED count must be {}",
        DELETED_COUNT
    );

    let write_count = db.tsl_query_count(&flash, from, to, FdbTslStatus::Write);
    assert_eq!(write_count, 0, "WRITE count must be 0 after set_status");
}

/// c: fdb_tsdb_tc.c:211-226 — test_fdb_tsl_clean
#[test]
fn test_c_equiv_clean() {
    let mut flash = EquivFlash::new(DB_SIZE as usize);
    let mut db = make_tsdb(&mut flash);
    append_tsls(&mut db, &mut flash, TSL_COUNT);

    // Clean
    db.tsl_clean(&mut flash);

    // Verify no TSLs
    let mut count = 0;
    db.tsl_iter(&flash, |_tsl| {
        count += 1;
        false
    });
    assert_eq!(count, 0, "iter after clean must return 0 TSLs");
    assert_eq!(db.last_time(), 0, "last_time must be 0 after clean");

    // Reboot and verify still clean
    db.deinit().unwrap();
    let mut db2 = make_tsdb(&mut flash);
    let mut count2 = 0;
    db2.tsl_iter(&flash, |_tsl| {
        count2 += 1;
        false
    });
    assert_eq!(count2, 0, "iter after reboot+clean must return 0 TSLs");

    // Verify can append after clean
    let mut data = 42i32.to_le_bytes();
    let mut blob = blob_make(&mut data);
    blob.size = 4;
    db2.tsl_append_with_ts(&mut flash, &blob, 100).unwrap();

    let mut count3 = 0;
    db2.tsl_iter(&flash, |tsl| {
        assert_eq!(tsl.time, 100);
        count3 += 1;
        false
    });
    assert_eq!(count3, 1, "iter after clean+append must return 1 TSL");
}

/// c: fdb_tsdb_tc.c:372-450 — test_fdb_tsl_iter_by_time_1 (multi-sector)
/// Verify iter_by_time works correctly across multiple sectors.
#[test]
fn test_c_equiv_iter_by_time_multi_sector() {
    let mut flash = EquivFlash::new(DB_SIZE as usize);
    let mut db = make_tsdb(&mut flash);

    // Append enough TSLs to span multiple sectors.
    // Each TSL: LOG_IDX_DATA_SIZE(16) + wg_align(4)(4) = 20 bytes
    // Per sector: (4096-32)/20 = 203 TSLs
    // Use 500 TSLs to span at least 3 sectors
    let multi_count = 500;
    append_tsls(&mut db, &mut flash, multi_count);

    // Full forward range
    let mut count = 0;
    let mut prev_time: FdbTime = 0;
    db.tsl_iter_by_time(&flash, 0, multi_count as FdbTime * TIME_STEP, |tsl| {
        assert!(tsl.time >= prev_time, "forward iter must be non-decreasing");
        prev_time = tsl.time;
        count += 1;
        false
    });
    assert_eq!(count, multi_count, "multi-sector forward iter must return all TSLs");

    // Full reverse range
    let mut count = 0;
    let mut prev_time = FdbTime::MAX;
    db.tsl_iter_by_time(
        &flash,
        multi_count as FdbTime * TIME_STEP,
        TIME_STEP,
        |tsl| {
            assert!(tsl.time <= prev_time, "reverse iter must be non-increasing");
            prev_time = tsl.time;
            count += 1;
            false
        },
    );
    assert_eq!(count, multi_count, "multi-sector reverse iter must return all TSLs");

    // Cross-sector range query
    let mid = multi_count / 2;
    let from = mid as FdbTime * TIME_STEP;
    let to = (mid + 10) as FdbTime * TIME_STEP;
    let mut count = 0;
    db.tsl_iter_by_time(&flash, from, to, |tsl| {
        assert!(tsl.time >= from && tsl.time <= to, "TSL must be in range");
        count += 1;
        false
    });
    assert_eq!(count, 11, "cross-sector range [{}..{}] must return 11 TSLs", from, to);
}

/// c: fdb_tsdb_tc.c:501-514 — max_blob_count sanity
#[test]
fn test_c_equiv_max_blob_count() {
    let mut flash = EquivFlash::new(DB_SIZE as usize);
    let db = make_tsdb(&mut flash);

    let max_count = db.tsl_max_blob_count();
    assert!(max_count > 0, "max_blob_count must be positive");

    // Verify we can append at least 1 TSL (max_blob_count is an upper bound)
    drop(db);
    let mut flash2 = EquivFlash::new(DB_SIZE as usize);
    let mut db2 = make_tsdb(&mut flash2);
    let mut data = 1i32.to_le_bytes();
    let mut blob = blob_make(&mut data);
    blob.size = 4;
    db2.tsl_append_with_ts(&mut flash2, &blob, 10).unwrap();
}
