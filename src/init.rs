// c: fdb.c — Initialize/deinitialize interface
//
// 1:1 Rust translation of fdb.c:31-157. The C `_fdb_init_ex` dispatched on
// `db->file_mode` to either FAL (`fal_partition_find`/`fal_flash_device_find`)
// or file mode. In the trait-based Rust port the flash backend is supplied via
// `FlashDevice`, so the FAL/file lookup is replaced by direct field validation.
// The caller is responsible for configuring `sec_size` and `max_size` on the
// `FdbDb` before calling `init_ex` (in C, FAL set these from the partition).

use core::sync::atomic::{AtomicBool, Ordering};

use crate::def::{FdbDb, FdbDbType, FdbErr};

/// c: fdb.c:104 — `static bool log_is_show = false;`
///
/// Tracks whether the "FlashDB initialized successfully" banner has already been
/// emitted, mirroring the C static. The actual log output is a no-op in `no_std`;
/// the state is preserved for behavioural fidelity.
static LOG_IS_SHOW: AtomicBool = AtomicBool::new(false);

/// c: fdb.c:31-100 — _fdb_init_ex
///
/// Validate and initialise the base database. The caller must set `sec_size` and
/// `max_size` on `db` beforehand (in C, FAL derived `max_size` from the partition
/// and defaulted `sec_size` to the flash block size).
///
/// Returns `Ok(())` when the configuration is valid, or:
/// - `Err(InitFailed)` when `max_size` is not a multiple of `sec_size`
/// - `Err(InitFailed)` when there are fewer than 2 sectors
/// - panics (assert) when `sec_size` is 0 or not a power of two (matches C's
///   `FDB_ASSERT((db->sec_size & (db->sec_size - 1)) == 0)`)
pub fn init_ex(
    db: &mut FdbDb,
    name: &'static str,
    path: &'static str,
    db_type: FdbDbType,
) -> Result<(), FdbErr> {
    // c: FDB_ASSERT(db); FDB_ASSERT(name); FDB_ASSERT(path);

    // c: if (db->init_ok) return FDB_NO_ERR;
    if db.init_ok {
        return Ok(());
    }

    // c: db->name = name; db->type = type; db->user_data = user_data;
    db.name = name;
    db.path = path;
    db.type_ = db_type;

    // The FAL/file branches that populated sec_size/max_size from the partition
    // are absent in the trait-based port; the caller configures them directly.
    // The remaining validation mirrors fdb.c:87-97 verbatim.

    // c: fdb.c:87 — the block size MUST be the Nth power of 2
    assert!(
        db.sec_size != 0 && (db.sec_size & (db.sec_size - 1)) == 0,
        "sec_size must be a non-zero power of two"
    );

    // c: fdb.c:89 — must align with sector size
    if !db.max_size.is_multiple_of(db.sec_size) {
        return Err(FdbErr::InitFailed);
    }

    // c: fdb.c:94 — must have more than or equal 2 sectors
    if db.max_size / db.sec_size < 2 {
        return Err(FdbErr::InitFailed);
    }

    Ok(())
}

/// c: fdb.c:102-116 — _fdb_init_finish
///
/// Mark the database as initialised on success. On failure, the C version logs
/// an error (suppressed when `not_formatable` is set); this translation preserves
/// the `init_ok`/`not_formatable` semantics without the log side effect.
pub fn init_finish(db: &mut FdbDb, result: Result<(), FdbErr>) {
    // c: if (result == FDB_NO_ERR) { db->init_ok = true; ... }
    if result.is_ok() {
        db.init_ok = true;
        // c: if (!log_is_show) { FDB_INFO(...); log_is_show = true; }
        if !LOG_IS_SHOW.swap(true, Ordering::SeqCst) {
            // Log output is a no-op in no_std; the once-only flag is preserved.
        }
    } else if !db.not_formatable {
        // c: FDB_INFO("Error: %s (%s@%s) is initialize fail (%d).\n", ...);
        // Log output is a no-op in no_std.
    }
}

/// c: fdb.c:118-139 — _fdb_deinit
///
/// Tear down the database. The C version closed cached file handles in file
/// mode; the trait-based port has no file cache, so only `init_ok` is cleared.
pub fn deinit(db: &mut FdbDb) {
    // c: FDB_ASSERT(db);
    // c: if (db->init_ok) { /* close file handles in file mode */ }
    // c: db->init_ok = false;
    db.init_ok = false;
}

/// c: fdb.c:141-157 — _fdb_db_path
///
/// Return the database path identifier. In C this returned the FAL partition
/// name or the file directory; here it returns the `path` stored by `init_ex`.
pub fn db_path(db: &FdbDb) -> &'static str {
    // c: return db->storage.part->name; (FAL) / db->storage.dir; (file)
    db.path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::def::{FdbDbType, FdbErr};

    fn make_db() -> FdbDb {
        FdbDb::default()
    }

    #[test]
    fn test_init_success() {
        // c: fdb.c scenario — name="test", sec_size=4096, max_size=16384 (4 sectors)
        let mut db = make_db();
        db.sec_size = 4096;
        db.max_size = 16384;

        let result = init_ex(&mut db, "test", "test_part", FdbDbType::Kv);
        assert!(result.is_ok(), "valid config should initialise");
        // init_ex does not set init_ok; init_finish does.
        assert!(!db.init_ok, "init_ok set only by init_finish");
        init_finish(&mut db, result);
        assert!(db.init_ok, "init_ok must be true after successful init_finish");
        assert_eq!(db.name, "test");
        assert_eq!(db.path, "test_part");
        assert_eq!(db.type_, FdbDbType::Kv);
    }

    #[test]
    fn test_init_already_initialised() {
        // c: fdb.c:37 — if (db->init_ok) return FDB_NO_ERR;
        let mut db = make_db();
        db.sec_size = 4096;
        db.max_size = 16384;
        db.init_ok = true;
        // Even with invalid sizes, init_ok short-circuits to Ok.
        db.sec_size = 0;
        let result = init_ex(&mut db, "test", "test_part", FdbDbType::Kv);
        assert!(result.is_ok(), "already-initialised db returns Ok immediately");
    }

    #[test]
    fn test_init_max_size_not_aligned() {
        // c: fdb.c:89 — max_size not a multiple of sec_size -> InitFailed
        let mut db = make_db();
        db.sec_size = 4096;
        db.max_size = 5000; // 5000 % 4096 != 0
        let result = init_ex(&mut db, "test", "test_part", FdbDbType::Kv);
        assert_eq!(result, Err(FdbErr::InitFailed));
        assert!(!db.init_ok, "failed init must not set init_ok");
    }

    #[test]
    fn test_init_single_sector() {
        // c: fdb.c:94 — fewer than 2 sectors -> InitFailed
        let mut db = make_db();
        db.sec_size = 4096;
        db.max_size = 4096; // exactly 1 sector
        let result = init_ex(&mut db, "test", "test_part", FdbDbType::Kv);
        assert_eq!(result, Err(FdbErr::InitFailed));
    }

    #[test]
    fn test_init_two_sectors_ok() {
        // Boundary: exactly 2 sectors is the minimum valid configuration.
        let mut db = make_db();
        db.sec_size = 4096;
        db.max_size = 8192;
        let result = init_ex(&mut db, "test", "test_part", FdbDbType::Ts);
        assert!(result.is_ok(), "exactly 2 sectors should be accepted");
        assert_eq!(db.type_, FdbDbType::Ts);
    }

    #[test]
    #[should_panic(expected = "sec_size must be a non-zero power of two")]
    fn test_init_sec_size_not_power_of_two() {
        // c: fdb.c:87 — FDB_ASSERT((db->sec_size & (db->sec_size - 1)) == 0)
        let mut db = make_db();
        db.sec_size = 3000; // not a power of two
        db.max_size = 12000;
        let _ = init_ex(&mut db, "test", "test_part", FdbDbType::Kv);
    }

    #[test]
    #[should_panic(expected = "sec_size must be a non-zero power of two")]
    fn test_init_sec_size_zero() {
        // sec_size == 0 is invalid (would also underflow the power-of-two check).
        let mut db = make_db();
        db.sec_size = 0;
        db.max_size = 8192;
        let _ = init_ex(&mut db, "test", "test_part", FdbDbType::Kv);
    }

    #[test]
    fn test_init_finish_failure_keeps_init_ok_false() {
        let mut db = make_db();
        db.sec_size = 4096;
        db.max_size = 16384;
        init_finish(&mut db, Err(FdbErr::InitFailed));
        assert!(!db.init_ok, "failed init_finish must not set init_ok");
    }

    #[test]
    fn test_init_finish_failure_not_formatable_silent() {
        // not_formatable suppresses the failure log; init_ok still stays false.
        let mut db = make_db();
        db.not_formatable = true;
        init_finish(&mut db, Err(FdbErr::ReadErr));
        assert!(!db.init_ok);
    }

    #[test]
    fn test_deinit_clears_init_ok() {
        let mut db = make_db();
        db.init_ok = true;
        deinit(&mut db);
        assert!(!db.init_ok, "deinit must clear init_ok");
    }

    #[test]
    fn test_deinit_idempotent_on_uninit() {
        let mut db = make_db();
        // deinit on an uninitialised db is a no-op (init_ok already false).
        deinit(&mut db);
        assert!(!db.init_ok);
    }

    #[test]
    fn test_db_path_returns_stored_path() {
        let mut db = make_db();
        db.sec_size = 4096;
        db.max_size = 16384;
        init_ex(&mut db, "kvdb", "fdb_kvdb1", FdbDbType::Kv).unwrap();
        assert_eq!(db_path(&db), "fdb_kvdb1");
    }
}
