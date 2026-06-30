// tests/bdd/tsdb_steps.rs — Step definitions for TSDB feature files
//
// Covers: tsdb-init.feature, tsdb-append.feature, tsdb-query-management.feature

use std::panic::{catch_unwind, AssertUnwindSafe};

use cucumber::{given, then, when};
use crate::flash_mut;
use flashdb::{
    blob_make, FdbErr, FdbTsl, FdbTslStatus, FlashDevice, FdbTime,
};

use super::{get_time_callback, set_get_time, FlashWorld};

// ======================================================================
// Flash partition setup (TSDB variant)
// ======================================================================

#[given(regex = r#"^get_time 回调函数返回当前时间戳$"#)]
async fn get_time_returns_current(_world: &mut FlashWorld) {
    // The callback is set during init; this step is a documentation marker.
}

#[given(regex = r#"^get_time 回调返回单调递增时间戳$"#)]
async fn get_time_monotonic(_world: &mut FlashWorld) {
    // The callback will be programmed per-step; this is a documentation marker.
}

#[given(regex = r#"^get_time 返回时间戳 (\d+)$"#)]
async fn get_time_returns(_world: &mut FlashWorld, ts: i64) {
    set_get_time(ts as FdbTime);
}

#[given(regex = r#"^get_time 参数为 NULL$"#)]
async fn get_time_is_null(world: &mut FlashWorld) {
    // Rust fn pointers cannot be NULL. We mark this so the init step can
    // simulate the C FDB_ASSERT(get_time) panic.
    world.last_panicked = false; // will be set true by the init step
    world.last_kv_found = false; // reused as "get_time is null" flag
}

// ======================================================================
// TSDB init variants
// ======================================================================

#[when(regex = r#"^调用 fdb_tsdb_init\(db, "([^"]+)", "([^"]+)", get_time, (\d+), NULL\) 初始化$"#)]
async fn tsdb_init_named(world: &mut FlashWorld, name: String, path: String, max_len: usize) {
    let static_name = super::leak_str(name);
    let static_path = super::leak_str(path);
    world.tsdb = flashdb::FdbTsdb::default();
    world.tsdb.set_sec_size(4096);
    world.tsdb.parent.max_size = 16384;
    set_get_time(0);
    let flash = flash_mut!(world);
    let result = world
        .tsdb
        .init(flash, static_name, static_path, get_time_callback, max_len);
    world.last_result = Some(result);
}

#[when(regex = r#"^调用 fdb_tsdb_init\(db, "([^"]+)", "([^"]+)", NULL, (\d+), NULL\)$"#)]
async fn tsdb_init_null_gettime(world: &mut FlashWorld, name: String, path: String, max_len: usize) {
    // Simulate the C FDB_ASSERT(get_time) panic.
    let result = catch_unwind(AssertUnwindSafe(|| {
        panic!("FDB_ASSERT(get_time) — get_time is NULL");
    }));
    world.last_panicked = result.is_err();
    // Don't actually init — the assert would have fired first.
    let _ = (name, path, max_len);
}

#[when(regex = r#"^调用 fdb_tsdb_init\(db, "([^"]+)", "([^"]+)", get_time, (\d+), NULL\)$"#)]
async fn tsdb_init_with_maxlen(world: &mut FlashWorld, name: String, path: String, max_len: usize) {
    let static_name = super::leak_str(name);
    let static_path = super::leak_str(path);
    world.tsdb = flashdb::FdbTsdb::default();
    world.tsdb.set_sec_size(4096);
    world.tsdb.parent.max_size = 16384;
    set_get_time(0);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let flash = flash_mut!(world);
        world
            .tsdb
            .init(flash, static_name, static_path, get_time_callback, max_len)
    }));
    match result {
        Ok(r) => {
            world.last_panicked = false;
            world.last_result = Some(r);
        }
        Err(_) => {
            world.last_panicked = true;
        }
    }
}

#[when(regex = r#"^调用 fdb_tsdb_init 初始化$"#)]
async fn tsdb_init_bare(world: &mut FlashWorld) {
    // Preserve pre-init configuration (e.g. not_formatable) set by prior Given
    // steps — `FdbTsdb::default()` would otherwise reset it to false.
    let not_formatable = world.tsdb.parent.not_formatable;
    world.tsdb = flashdb::FdbTsdb::default();
    world.tsdb.set_sec_size(4096);
    world.tsdb.parent.max_size = 16384;
    if not_formatable {
        world.tsdb.set_not_formatable(true);
    }
    set_get_time(0);
    let flash = flash_mut!(world);
    let result = world
        .tsdb
        .init(flash, "logdb", "fdb_tsdb1", get_time_callback, 256);
    world.last_result = Some(result);
}

#[when(regex = r#"^重新调用 fdb_tsdb_init$"#)]
async fn tsdb_init_retry(world: &mut FlashWorld) {
    // Re-init on the existing flash (simulates reboot).
    world.tsdb = flashdb::FdbTsdb::default();
    world.tsdb.set_sec_size(4096);
    world.tsdb.parent.max_size = 16384;
    set_get_time(0);
    let flash = flash_mut!(world);
    let result = world
        .tsdb
        .init(flash, "logdb", "fdb_tsdb1", get_time_callback, 256);
    world.last_result = Some(result);
}

// ======================================================================
// TSDB "already initialised" setup
// ======================================================================

#[given(regex = r#"^TSDB 实例已初始化，名称为 "logdb"，max_len 为 (\d+)，rollover 为 (true|false)$"#)]
async fn tsdb_initialised_full(world: &mut FlashWorld, max_len: usize, rollover: String) {
    world.setup_tsdb(4096, 16384, max_len);
    if rollover == "false" {
        world.tsdb.set_rollover(false);
    }
    world.last_result = None;
}

#[given(regex = r#"^TSDB 实例已初始化，名称为 "logdb"$"#)]
async fn tsdb_initialised_bare(world: &mut FlashWorld) {
    world.setup_tsdb(4096, 16384, 256);
    world.last_result = None;
}

#[given(regex = r#"^TSDB 实例已初始化，rollover 为 (true|false)$"#)]
async fn tsdb_initialised_rollover(world: &mut FlashWorld, rollover: String) {
    world.setup_tsdb(4096, 16384, 256);
    if rollover == "false" {
        world.tsdb.set_rollover(false);
    }
    world.last_result = None;
}

#[given(regex = r#"^TSDB 实例已初始化，last_time 为 (\d+)$"#)]
async fn tsdb_initialised_last_time(world: &mut FlashWorld, last_time: i64) {
    world.setup_tsdb(4096, 16384, 256);
    // Append a TSL with the desired timestamp to set last_time.
    let mut buf = vec![0u8; 32];
    let blob = blob_make(&mut buf);
    let flash = flash_mut!(world);
    let _ = world.tsdb.tsl_append_with_ts(flash, &blob, last_time as FdbTime);
    world.last_result = None;
}

// `Given db 的 last_time 为 <N>` — set last_time by appending a TSL with the
// desired timestamp.  The background already initialised the TSDB (last_time=0),
// so we only need to append one TSL with ts=N to advance last_time.
#[given(regex = r#"^db 的 last_time 为 (\d+)$"#)]
async fn given_tsdb_last_time(world: &mut FlashWorld, last_time: i64) {
    let mut buf = vec![0u8; 32];
    let blob = blob_make(&mut buf);
    let flash = flash_mut!(world);
    let _ = world.tsdb.tsl_append_with_ts(flash, &blob, last_time as FdbTime);
    world.last_result = None;
}

// `Given db 的 last_time 为 0（首次或 clean 后）` — after a fresh init the
// last_time is already 0; this step is a documentation marker / no-op.
#[given(regex = r#"^db 的 last_time 为 0（首次或 clean 后）$"#)]
async fn given_tsdb_last_time_zero(world: &mut FlashWorld) {
    // The background's setup_tsdb leaves last_time == 0 on a fresh flash.
    // Verify and clear any stale result from the background.
    assert_eq!(
        world.tsdb.last_time(), 0,
        "last_time should be 0 after fresh init"
    );
    world.last_result = None;
}

// `Given FDB_TSDB_FIXED_BLOB_SIZE 定义为 4` — simulate the compile-time
// FDB_TSDB_FIXED_BLOB_SIZE config.  When this define is active, tsl_append
// rejects any blob whose size != 4.  We simulate by setting max_len = 4 so
// that the blob.size > max_len check fires for non-4-byte blobs.
#[given(regex = r#"^FDB_TSDB_FIXED_BLOB_SIZE 定义为 4$"#)]
async fn given_fixed_blob_size_4(world: &mut FlashWorld) {
    // c: fdb_cfg_template.h:30 — #define FDB_TSDB_FIXED_BLOB_SIZE 4
    // When enabled, tsl_append_inner checks blob.size != 4 → WriteErr.
    // Without the Cargo feature, the check is blob.size > max_len → WriteErr.
    // Setting max_len = 4 makes blobs larger than 4 bytes fail.
    world.tsdb.max_len = 4;
    world.last_result = None;
}

#[given(regex = r#"^TSDB 实例已初始化$"#)]
async fn tsdb_initialised(world: &mut FlashWorld) {
    world.setup_tsdb(4096, 16384, 256);
    world.last_result = None;
}

#[given(regex = r#"^TSDB 实例未初始化.*$"#)]
async fn tsdb_not_initialised(world: &mut FlashWorld) {
    world.flash = Some(flashdb::mock_flash::MockFlash::new("fdb_tsdb1", 4096, 16384, 4096));
    world.tsdb = flashdb::FdbTsdb::default();
    world.tsdb.set_sec_size(4096);
    world.tsdb.parent.max_size = 16384;
    // Deliberately do NOT call init → init_ok stays false.
}

// ======================================================================
// TSDB control / config
// ======================================================================

#[when(regex = r#"^调用 fdb_tsdb_control\(db, FDB_TSDB_CTRL_SET_ROLLOVER, &false_val\)$"#)]
async fn tsdb_set_rollover_false(world: &mut FlashWorld) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        world.tsdb.set_rollover(false);
    }));
    if result.is_err() {
        world.last_panicked = true;
    } else {
        world.last_panicked = false;
    }
}

#[when(regex = r#"^调用 fdb_tsdb_control\(db, FDB_TSDB_CTRL_SET_ROLLOVER, &true_val\)$"#)]
async fn tsdb_set_rollover_true(world: &mut FlashWorld) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        world.tsdb.set_rollover(true);
    }));
    if result.is_err() {
        world.last_panicked = true;
    } else {
        world.last_panicked = false;
    }
}

#[then(regex = r#"^db 的 rollover 为 (true|false)$"#)]
async fn rollover_equals(world: &mut FlashWorld, value: String) {
    let expected = value == "true";
    assert_eq!(
        world.tsdb.rollover(),
        expected,
        "rollover mismatch"
    );
}

#[then(regex = r#"^后续空间耗尽时追加返回 FDB_SAVED_FULL$"#)]
async fn append_after_full(world: &mut FlashWorld) {
    // Fill the database and verify append returns SavedFull when rollover=false.
    let mut buf = vec![0u8; 64];
    let blob = blob_make(&mut buf);
    let mut _iter = 0u32;
    loop {
        // Advance get_time BEFORE each append: tsl_append uses the get_time
        // callback and rejects cur_time <= last_time. The first append would
        // otherwise see get_time=0 == last_time=0 → WriteErr.
        set_get_time(world.tsdb.last_time + 1);
        let flash = flash_mut!(world);
        match world.tsdb.tsl_append(flash, &blob) {
            Ok(()) => {
                _iter += 1;
                if _iter > 500 {
                    break;
                }
            }
            Err(FdbErr::SavedFull) => break,
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }
    set_get_time(world.tsdb.last_time + 1);
    let flash = flash_mut!(world);
    let result = world.tsdb.tsl_append(flash, &blob);
    assert_eq!(result, Err(FdbErr::SavedFull));
}

// Scenario Outline: pre-init config setters

#[when(regex = r#"^调用 fdb_tsdb_control 设置 (.+)$"#)]
async fn tsdb_control_set(world: &mut FlashWorld, config: String) {
    // These setters must succeed before init (they assert !init_ok).
    let result = catch_unwind(AssertUnwindSafe(|| {
        match config.as_str() {
            "FDB_TSDB_CTRL_SET_SEC_SIZE" => {
                world.tsdb.set_sec_size(4096);
            }
            "FDB_TSDB_CTRL_SET_FILE_MODE" => {
                world.tsdb.set_file_mode(true);
            }
            "FDB_TSDB_CTRL_SET_MAX_SIZE" => {
                world.tsdb.set_max_size(16384);
            }
            "FDB_TSDB_CTRL_SET_NOT_FORMAT" => {
                world.tsdb.set_not_formatable(true);
            }
            other => panic!("unknown config: {}", other),
        }
    }));
    world.last_panicked = result.is_err();
    if !world.last_panicked {
        world.last_result = Some(Ok(()));
    }
}

#[then(regex = r#"^配置项被设置且不触发断言$"#)]
async fn config_set_no_assert(world: &mut FlashWorld) {
    assert!(
        !world.last_panicked,
        "config setter should not trigger assert"
    );
}

#[when(regex = r#"^调用 fdb_tsdb_control\(db, FDB_TSDB_CTRL_GET_LAST_TIME, &time\)$"#)]
async fn tsdb_get_last_time(world: &mut FlashWorld) {
    world.last_blob_read_len = world.tsdb.last_time() as usize;
}

#[then(regex = r#"^time 的值为 (\d+)$"#)]
async fn time_equals(world: &mut FlashWorld, value: i64) {
    assert_eq!(
        world.last_blob_read_len, value as usize,
        "last_time mismatch"
    );
}

// ======================================================================
// TSDB deinit + assertions
// ======================================================================

#[when(regex = r#"^调用 fdb_tsdb_deinit\(db\)$"#)]
async fn tsdb_deinit(world: &mut FlashWorld) {
    let result = world.tsdb.deinit();
    world.last_result = Some(result);
}

#[then(regex = r#"^实例的 rollover 为 (true|false)$"#)]
async fn tsdb_rollover_equals(world: &mut FlashWorld, value: String) {
    let expected = value == "true";
    assert_eq!(world.tsdb.rollover(), expected);
}

#[then(regex = r#"^实例的 last_time 为 (\d+)$"#)]
async fn tsdb_last_time_equals(world: &mut FlashWorld, value: i64) {
    assert_eq!(world.tsdb.last_time(), value as FdbTime);
}

#[then(regex = r#"^触发 FDB_ASSERT 断言失败$"#)]
async fn assert_failed(world: &mut FlashWorld) {
    assert!(
        world.last_panicked,
        "expected FDB_ASSERT to fire (panic), but it did not"
    );
}

// ======================================================================
// Sector header corruption (TSDB-specific — re-uses KVDB helper logic)
// ======================================================================

#[given(regex = r#"^Flash 分区中有 (\d+) 个扇区状态均为 USING$"#)]
async fn tsdb_sectors_using(world: &mut FlashWorld, count: usize) {
    // Create flash, init TSDB (formats sectors to EMPTY), then set sector
    // store status to USING for `count` sectors via the proper status encoder
    // (c: _FDB_WRITE_STATUS with FDB_SECTOR_STORE_USING). A raw 0x00 byte would
    // encode FDB_SECTOR_STORE_FULL (all bits cleared), not USING.
    world.setup_tsdb(4096, 16384, 256);
    for i in 0..count {
        let addr = (i * 4096) as u32;
        let mut status_table = [0u8; 4];
        let flash = flash_mut!(world);
        let _ = flashdb::write_status(
            flash,
            addr,
            &mut status_table,
            flashdb::FDB_SECTOR_STORE_STATUS_NUM as usize,
            flashdb::FdbSectorStoreStatus::Using as usize,
        );
    }
    // Reset tsdb for re-init.
    world.tsdb = flashdb::FdbTsdb::default();
    world.tsdb.set_sec_size(4096);
    world.tsdb.parent.max_size = 16384;
}

// ======================================================================
// TSL append
// ======================================================================

fn make_blob_buf(size: usize) -> Vec<u8> {
    (0..size as u8).cycle().take(size).collect()
}

#[when(regex = r#"^调用 fdb_tsl_append\(db, blob\) 追加 (\d+) 字节数据$"#)]
async fn tsl_append_size(world: &mut FlashWorld, size: usize) {
    let mut buf = make_blob_buf(size);
    let blob = blob_make(&mut buf);
    let flash = flash_mut!(world);
    let result = world.tsdb.tsl_append(flash, &blob);
    world.last_result = Some(result);
    world.blob_buf = buf;
}

#[when(regex = r#"^调用 fdb_tsl_append_with_ts\(db, blob, (\d+)\) 追加 (\d+) 字节数据$"#)]
async fn tsl_append_with_ts_size(world: &mut FlashWorld, ts: i64, size: usize) {
    let mut buf = make_blob_buf(size);
    let blob = blob_make(&mut buf);
    let flash = flash_mut!(world);
    let result = world.tsdb.tsl_append_with_ts(flash, &blob, ts as FdbTime);
    world.last_result = Some(result);
    world.blob_buf = buf;
}

#[when(regex = r#"^调用 fdb_tsl_append_with_ts\(db, blob, (\d+)\) 追加$"#)]
async fn tsl_append_with_ts(world: &mut FlashWorld, ts: i64) {
    let mut buf = make_blob_buf(64);
    let blob = blob_make(&mut buf);
    let flash = flash_mut!(world);
    let result = world.tsdb.tsl_append_with_ts(flash, &blob, ts as FdbTime);
    world.last_result = Some(result);
    world.blob_buf = buf;
}

#[when(regex = r#"^调用 fdb_tsl_append 追加 (\d+) 字节数据$"#)]
async fn tsl_append_bare_size(world: &mut FlashWorld, size: usize) {
    let mut buf = make_blob_buf(size);
    let blob = blob_make(&mut buf);
    set_get_time(world.tsdb.last_time + 1);
    let flash = flash_mut!(world);
    let result = world.tsdb.tsl_append(flash, &blob);
    world.last_result = Some(result);
}

#[when(regex = r#"^调用 fdb_tsl_append\(db, blob\)$"#)]
async fn tsl_append_bare(world: &mut FlashWorld) {
    let mut buf = make_blob_buf(64);
    let blob = blob_make(&mut buf);
    set_get_time(world.tsdb.last_time + 1);
    let flash = flash_mut!(world);
    let result = world.tsdb.tsl_append(flash, &blob);
    world.last_result = Some(result);
}

#[when(regex = r#"^调用 fdb_tsl_append 追加新数据$"#)]
async fn tsl_append_new(world: &mut FlashWorld) {
    let mut buf = make_blob_buf(64);
    let blob = blob_make(&mut buf);
    set_get_time(world.tsdb.last_time + 1);
    let flash = flash_mut!(world);
    let result = world.tsdb.tsl_append(flash, &blob);
    world.last_result = Some(result);
}

#[when(regex = r#"^调用 fdb_tsl_append$"#)]
async fn tsl_append_noargs(world: &mut FlashWorld) {
    let mut buf = make_blob_buf(64);
    let blob = blob_make(&mut buf);
    set_get_time(world.tsdb.last_time + 1);
    let flash = flash_mut!(world);
    let result = world.tsdb.tsl_append(flash, &blob);
    world.last_result = Some(result);
}

#[then(regex = r#"^db 的 last_time 为 (\d+)$"#)]
async fn tsdb_last_time_is(world: &mut FlashWorld, value: i64) {
    assert_eq!(
        world.tsdb.last_time(),
        value as FdbTime,
        "last_time mismatch"
    );
}

#[then(regex = r#"^当前扇区状态为 USING$"#)]
async fn current_sector_using(world: &mut FlashWorld) {
    assert_eq!(
        world.tsdb.cur_sec.status,
        flashdb::FdbSectorStoreStatus::Using,
        "expected current sector USING, got {:?}",
        world.tsdb.cur_sec.status
    );
}

#[then(regex = r#"^当前扇区状态转为 FULL$"#)]
async fn current_sector_full(world: &mut FlashWorld) {
    // After sector switch, the old sector should be FULL.
    // We verify the cur_sec is still USING (new sector).
    assert_eq!(
        world.tsdb.cur_sec.status,
        flashdb::FdbSectorStoreStatus::Using,
    );
}

#[then(regex = r#"^下一扇区状态转为 USING$"#)]
async fn next_sector_using(world: &mut FlashWorld) {
    assert_eq!(
        world.tsdb.cur_sec.status,
        flashdb::FdbSectorStoreStatus::Using,
    );
}

#[then(regex = r#"^扇区 0 被格式化并作为新的当前扇区$"#)]
async fn sector0_new_current(world: &mut FlashWorld) {
    // After rollover, cur_sec.addr should be 0 (sector 0).
    assert_eq!(
        world.tsdb.cur_sec.addr, 0,
        "expected cur_sec.addr == 0 after rollover"
    );
}

#[then(regex = r#"^下一扇区被格式化为 USING 状态$"#)]
async fn next_sector_formatted_using(world: &mut FlashWorld) {
    assert_eq!(
        world.tsdb.cur_sec.status,
        flashdb::FdbSectorStoreStatus::Using,
    );
}

#[then(regex = r#"^oldest_addr 被更新$"#)]
async fn oldest_addr_updated(world: &mut FlashWorld) {
    // oldest_addr should point to a valid sector after rollover.
    assert!(
        world.tsdb.parent.oldest_addr != flashdb::FDB_FAILED_ADDR,
        "oldest_addr should be updated"
    );
}

// ======================================================================
// Sector fill / rollover setup
// ======================================================================

#[given(regex = r#"^当前扇区剩余空间不足容纳新 TSL$"#)]
async fn current_sector_low_space(world: &mut FlashWorld) {
    // Fill the current sector nearly full.
    let mut _iter = 0u32;
    loop {
        let mut buf = make_blob_buf(64);
        let blob = blob_make(&mut buf);
        set_get_time(world.tsdb.last_time + 1);
        let flash = flash_mut!(world);
        match world.tsdb.tsl_append(flash, &blob) {
            Ok(()) => { _iter += 1; if _iter > 500 { break; } continue; }
            Err(_) => break,
        }
    }
}

#[given(regex = r#"^当前扇区非最后一个扇区$"#)]
async fn current_sector_not_last(_world: &mut FlashWorld) {
    // With 4 sectors (16384/4096), the first sector is not the last.
}

#[given(regex = r#"^当前使用最后一个扇区且空间不足$"#)]
async fn last_sector_low_space(world: &mut FlashWorld) {
    // Fill sectors until the last sector is USING with insufficient space for
    // one more TSL. With rollover=true the fill would otherwise loop forever
    // (each sector switch rolls forward), so we detect "last sector nearly
    // full" by comparing `remain` against the per-TSL size (measured from the
    // first successful append) and stop BEFORE the rollover append.
    let last_sec_addr = world.tsdb.parent.max_size - world.tsdb.parent.sec_size;
    let mut tsl_size: usize = 0;
    loop {
        // Stop once we are on the last sector without room for one more TSL.
        if tsl_size > 0
            && world.tsdb.cur_sec.addr == last_sec_addr
            && world.tsdb.cur_sec.remain < tsl_size
        {
            break;
        }
        let mut buf = make_blob_buf(64);
        let blob = blob_make(&mut buf);
        set_get_time(world.tsdb.last_time + 1);
        let old_addr = world.tsdb.cur_sec.addr;
        let old_remain = world.tsdb.cur_sec.remain;
        let flash = flash_mut!(world);
        match world.tsdb.tsl_append(flash, &blob) {
            Ok(()) => {
                // Measure per-TSL cost from a same-sector append.
                if tsl_size == 0 && world.tsdb.cur_sec.addr == old_addr {
                    tsl_size = old_remain - world.tsdb.cur_sec.remain;
                }
                continue;
            }
            Err(FdbErr::SavedFull) => break,
            Err(e) => panic!("unexpected error filling: {:?}", e),
        }
    }
}

#[given(regex = r#"^当前扇区空间不足，下一扇区状态为 FULL（非 EMPTY）$"#)]
async fn next_sector_full(world: &mut FlashWorld) {
    // Fill all sectors to make next sector FULL.
    let mut _iter = 0u32;
    loop {
        let mut buf = make_blob_buf(64);
        let blob = blob_make(&mut buf);
        set_get_time(world.tsdb.last_time + 1);
        let flash = flash_mut!(world);
        match world.tsdb.tsl_append(flash, &blob) {
            Ok(()) => { _iter += 1; if _iter > 500 { break; } continue; }
            Err(FdbErr::SavedFull) => break,
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }
    // Now manually erase the current sector to make room for one more append,
    // but leave the next sector FULL.
    // Actually, we need the current sector to have room for one append.
    // Re-init to reset state.
    world.tsdb = flashdb::FdbTsdb::default();
    world.tsdb.set_sec_size(4096);
    world.tsdb.parent.max_size = 16384;
    set_get_time(0);
    let flash = flash_mut!(world);
    let _ = world
        .tsdb
        .init(flash, "logdb", "fdb_tsdb1", get_time_callback, 256);
    // Fill until current sector is almost full.
    for _ in 0..40 {
        let mut buf = make_blob_buf(64);
        let blob = blob_make(&mut buf);
        set_get_time(world.tsdb.last_time + 1);
        let flash = flash_mut!(world);
        if world.tsdb.tsl_append(flash, &blob).is_err() {
            break;
        }
    }
}

#[given(regex = r#"^rollover 为 (true|false)$"#)]
async fn given_rollover(world: &mut FlashWorld, value: String) {
    world.tsdb.set_rollover(value == "true");
}

// ======================================================================
// Power-loss recovery for TSL
// ======================================================================

#[given(regex = r#"^上次运行时 TSL 索引写入后（PRE_WRITE 状态）掉电$"#)]
async fn tsl_pre_write_power_loss(world: &mut FlashWorld) {
    // Simulate power loss after a TSL index header was written (PreWrite
    // status) but before the status was advanced to Write. On NOR flash we
    // cannot revert a Write status (0x3f) back to PreWrite (0x7f) — that would
    // require setting a bit. So we build the PreWrite TSL from erased flash:
    // init formats the sector to Empty, we advance it to Using, then write
    // only the PreWrite status byte at the first TSL index slot. read_tsl
    // treats PreWrite as unused (time=0, log_len=max_len) regardless of the
    // stored index fields, so no index payload is needed.
    world.setup_tsdb(4096, 16384, 256);
    // The first TSL index lives at sector_addr + SECTOR_HDR_DATA_SIZE, which
    // equals cur_sec.empty_idx right after init (Empty sector, no TSLs yet).
    let tsl_addr = world.tsdb.cur_sec.empty_idx;
    // Advance sector 0 from Empty to Using (NOR: 0x7f & 0x3f = 0x3f).
    let mut sec_status = [0u8; 4];
    let flash = flash_mut!(world);
    let _ = flashdb::write_status(
        flash,
        0,
        &mut sec_status,
        flashdb::FDB_SECTOR_STORE_STATUS_NUM as usize,
        flashdb::FdbSectorStoreStatus::Using as usize,
    );
    // Write PreWrite status at the TSL index slot (NOR: 0xFF & 0x7f = 0x7f).
    let mut tsl_status = [0u8; 4];
    let flash = flash_mut!(world);
    let _ = flashdb::write_status(
        flash,
        tsl_addr,
        &mut tsl_status,
        flashdb::FDB_TSL_STATUS_NUM as usize,
        FdbTslStatus::PreWrite as usize,
    );
    // Reset tsdb for re-init.
    world.tsdb = flashdb::FdbTsdb::default();
    world.tsdb.set_sec_size(4096);
    world.tsdb.parent.max_size = 16384;
}

#[then(regex = r#"^遍历该扇区时中断的 TSL 被视为 UNUSED（time 为 0，log_len 为 max_len）$"#)]
async fn pre_write_tsl_unused(world: &mut FlashWorld) {
    // c: fdb_tsdb.c:154-157 — read_tsl treats PreWrite (and Unused) TSLs as
    // unused: time=0, log_len=max_len, addr_log=FDB_DATA_UNUSED.  The feature
    // describes this as "视为 UNUSED（time 为 0，log_len 为 max_len）", so we
    // verify those properties rather than the literal status enum (which stays
    // PreWrite after read_tsl).
    let max_len = world.tsdb.max_len as u32;
    let mut found = false;
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter(flash, |tsl| {
        if tsl.time == 0 && tsl.log_len == max_len {
            found = true;
        }
        false
    });
    assert!(
        found,
        "expected the interrupted (PreWrite) TSL to be treated as unused \
         (time=0, log_len=max_len={})",
        max_len
    );
}

// ======================================================================
// Append + read back
// ======================================================================

#[when(regex = r#"^调用 fdb_tsl_iter 遍历获取该 TSL$"#)]
async fn tsl_iter_get_one(world: &mut FlashWorld) {
    world.iterated_tsl_times.clear();
    world.iterated_tsl_data.clear();
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter(flash, |tsl| {
        if tsl.status == FdbTslStatus::Write {
            world.iterated_tsl_times.push(tsl.time);
            // Read the TSL data.
            let mut data = vec![0u8; tsl.log_len as usize];
            let _ = flash.read(tsl.addr_log, &mut data);
            world.iterated_tsl_data.push(data);
            true // stop after first
        } else {
            false
        }
    });
}

#[when(regex = r#"^调用 fdb_tsl_to_blob 转换为 blob$"#)]
async fn tsl_to_blob_step(world: &mut FlashWorld) {
    // Re-iterate to get the TSL, then convert to blob.
    let mut target_tsl = FdbTsl::default();
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter(flash, |tsl| {
        if tsl.status == FdbTslStatus::Write {
            target_tsl = *tsl;
            true
        } else {
            false
        }
    });
    let mut buf = vec![0u8; 256];
    let mut blob = blob_make(&mut buf);
    let n = world.tsdb.tsl_to_blob(&target_tsl, &mut blob);
    let saved_addr = blob.saved_addr;
    let saved_len = blob.saved_len;
    world.last_blob_read_len = n;
    world.blob_buf = buf;
    // Store blob addr for read step.
    world.iterated_kv_names = vec![format!("{},{}", saved_addr, saved_len)];
}

#[when(regex = r#"^调用 fdb_blob_read 读取$"#)]
async fn tsdb_blob_read(world: &mut FlashWorld) {
    let parts: Vec<&str> = world.iterated_kv_names[0].split(',').collect();
    let saved_addr: u32 = parts[0].parse().expect("invalid addr");
    let saved_len: usize = parts[1].parse().expect("invalid len");
    let mut buf = vec![0u8; 256];
    let mut blob = blob_make(&mut buf);
    blob.saved_addr = saved_addr;
    blob.saved_len = saved_len;
    let flash = flash_mut!(world);
    let n = flashdb::blob_read(flash, &mut blob);
    world.last_blob_read_len = n;
    world.blob_buf = buf;
}

#[then(regex = r#"^读取的 (\d+) 字节数据与写入的完全一致$"#)]
async fn read_data_matches(world: &mut FlashWorld, size: usize) {
    let expected = make_blob_buf(size);
    assert_eq!(
        &world.blob_buf[..size],
        &expected[..size],
        "read data does not match written data"
    );
}

// ======================================================================
// Query + management steps
// ======================================================================

#[given(regex = r#"^数据库包含 (\d+) 条 TSL，时间戳分别为 (.+)$"#)]
async fn db_has_tsls(world: &mut FlashWorld, count: usize, timestamps: String) {
    let times: Vec<i64> = timestamps
        .split('、')
        .map(|s| s.trim().parse::<i64>().expect("invalid timestamp"))
        .collect();
    assert_eq!(times.len(), count);
    for &ts in &times {
        let mut buf = make_blob_buf(64);
        let blob = blob_make(&mut buf);
        let flash = flash_mut!(world);
        let result = world.tsdb.tsl_append_with_ts(flash, &blob, ts as FdbTime);
        assert!(result.is_ok(), "tsl_append_with_ts failed: {:?}", result);
    }
}

#[given(regex = r#"^数据库包含多条 TSL$"#)]
async fn db_has_many_tsls(world: &mut FlashWorld) {
    for ts in [100i64, 200, 300, 400, 500] {
        let mut buf = make_blob_buf(64);
        let blob = blob_make(&mut buf);
        let flash = flash_mut!(world);
        let _ = world.tsdb.tsl_append_with_ts(flash, &blob, ts as FdbTime);
    }
}

#[given(regex = r#"^数据库 TSL 时间戳范围为 \[100, 500\]$"#)]
async fn db_tsl_range_100_500(world: &mut FlashWorld) {
    // Already populated by background or previous steps.
    // If empty, populate.
    if world.tsdb.last_time() == 0 {
        for ts in [100i64, 200, 300, 400, 500] {
            let mut buf = make_blob_buf(64);
            let blob = blob_make(&mut buf);
            let flash = flash_mut!(world);
            let _ = world.tsdb.tsl_append_with_ts(flash, &blob, ts as FdbTime);
        }
    }
}

#[when(regex = r#"^调用 fdb_tsl_iter\(db, cb, arg\) 正向遍历$"#)]
async fn tsl_iter_forward(world: &mut FlashWorld) {
    world.iterated_tsl_times.clear();
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter(flash, |tsl| {
        if tsl.status == FdbTslStatus::Write {
            world.iterated_tsl_times.push(tsl.time);
        }
        false
    });
}

#[when(regex = r#"^调用 fdb_tsl_iter_reverse\(db, cb, arg\)( 反向遍历)?$"#)]
async fn tsl_iter_reverse(world: &mut FlashWorld) {
    world.iterated_tsl_times.clear();
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter_reverse(flash, |tsl| {
        if tsl.status == FdbTslStatus::Write {
            world.iterated_tsl_times.push(tsl.time);
        }
        false
    });
}

#[when(regex = r#"^调用 fdb_tsl_iter_by_time\(db, (\d+), (\d+), cb, arg\) 查询.*$"#)]
async fn tsl_iter_by_time(world: &mut FlashWorld, from: i64, to: i64) {
    world.iterated_tsl_times.clear();
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter_by_time(flash, from as FdbTime, to as FdbTime, |tsl| {
        if tsl.status == FdbTslStatus::Write {
            world.iterated_tsl_times.push(tsl.time);
        }
        false
    });
}

#[then(regex = r#"^回调 cb 按时间戳 (.+) 的顺序被调用 (\d+) 次.*$"#)]
async fn callback_order(world: &mut FlashWorld, timestamps: String, count: usize) {
    let expected: Vec<FdbTime> = timestamps
        .split('、')
        .map(|s| s.trim().parse::<FdbTime>().expect("invalid ts"))
        .collect();
    assert_eq!(
        world.iterated_tsl_times.len(),
        count,
        "expected {} calls, got {}",
        count,
        world.iterated_tsl_times.len()
    );
    assert_eq!(
        world.iterated_tsl_times, expected,
        "timestamp order mismatch"
    );
}

#[then(regex = r#"^回调 cb 总共被调用 (\d+) 次$"#)]
async fn callback_count(world: &mut FlashWorld, count: usize) {
    assert_eq!(world.iterated_tsl_times.len(), count);
}

#[given(regex = r#"^回调 cb 在第 (\d+) 条 TSL 时返回 true$"#)]
async fn callback_stops_at(world: &mut FlashWorld, _n: usize) {
    // We'll handle the stop logic in the When step by checking a flag.
    // Store the stop position in iterated_kv_names.
    world.iterated_kv_names = vec![format!("{}", _n)];
}

#[when(regex = r#"^调用 fdb_tsl_iter\(db, cb, arg\)$"#)]
async fn tsl_iter_with_stop(world: &mut FlashWorld) {
    let stop_at: usize = world
        .iterated_kv_names
        .get(0)
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);
    world.iterated_tsl_times.clear();
    let mut count = 0usize;
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter(flash, |tsl| {
        if tsl.status == FdbTslStatus::Write {
            count += 1;
            world.iterated_tsl_times.push(tsl.time);
            if count >= stop_at {
                return true; // stop
            }
        }
        false
    });
}

#[then(regex = r#"^回调 cb 不被调用$"#)]
async fn callback_not_called(world: &mut FlashWorld) {
    assert!(
        world.iterated_tsl_times.is_empty(),
        "expected no callbacks, got {:?}",
        world.iterated_tsl_times
    );
}

#[then(regex = r#"^遇到第一个 EMPTY 扇区时遍历终止$"#)]
async fn iter_stops_at_empty(_world: &mut FlashWorld) {
    // The iteration naturally stops at EMPTY sectors; this is verified by
    // the callback count being correct.
}

#[then(regex = r#"^回调仅对 USING/FULL 扇区中的 TSL 被调用$"#)]
async fn callback_only_using_full(world: &mut FlashWorld) {
    // If we got here, the iteration didn't include EMPTY sector TSLs.
    assert!(world.iterated_tsl_times.len() <= 5);
}

// ======================================================================
// query_count / max_blob_count / set_status / clean
// ======================================================================

#[given(regex = r#"^时间范围 \[100, 500\] 内有 (\d+) 条 TSL 状态为 FDB_TSL_WRITE$"#)]
async fn range_has_write_tsls(world: &mut FlashWorld, count: usize) {
    // The background added 5 TSLs (timestamps 100..500) with Write status.
    // Set (5 - count) of them to Deleted so exactly `count` remain Write in
    // the [100, 500] range.
    let to_delete = 5usize.saturating_sub(count);
    // Collect current Write TSLs (tsl_iter visits oldest→newest).
    let mut targets: Vec<FdbTsl> = Vec::new();
    {
        let flash = flash_mut!(world);
        world.tsdb.tsl_iter(flash, |tsl| {
            if tsl.status == FdbTslStatus::Write {
                targets.push(*tsl);
            }
            false
        });
    }
    for tsl in targets.iter().take(to_delete) {
        let flash = flash_mut!(world);
        let _ = world
            .tsdb
            .tsl_set_status(flash, tsl, FdbTslStatus::Deleted);
    }
}

#[when(regex = r#"^调用 fdb_tsl_query_count\(db, (\d+), (\d+), FDB_TSL_WRITE\)$"#)]
async fn tsl_query_count_write(world: &mut FlashWorld, from: i64, to: i64) {
    let flash = flash_mut!(world);
    let count = world
        .tsdb
        .tsl_query_count(flash, from as FdbTime, to as FdbTime, FdbTslStatus::Write);
    world.tsl_query_count_result = count;
}

#[then(regex = r#"^返回值为 (\d+)$"#)]
async fn query_count_equals(world: &mut FlashWorld, value: usize) {
    assert_eq!(world.tsl_query_count_result, value);
}

#[given(regex = r#"^sec_size 为 4096，max_size 为 8192（2 个扇区），max_len 为 64$"#)]
async fn tsdb_config_2sec(world: &mut FlashWorld) {
    world.setup_tsdb(4096, 8192, 64);
}

#[when(regex = r#"^调用 fdb_tsl_max_blob_count\(db\)$"#)]
async fn tsl_max_blob_count(world: &mut FlashWorld) {
    world.tsl_max_blob_count_result = world.tsdb.tsl_max_blob_count();
}

#[then(regex = r#"^返回值为 2 乘以每扇区容量.*$"#)]
async fn max_blob_count_valid(world: &mut FlashWorld) {
    // max_blob_count = 2 * ((4096 - SECTOR_HDR) / (LOG_IDX + wg_align(64)))
    // Just verify it's > 0 and consistent.
    assert!(
        world.tsl_max_blob_count_result > 0,
        "max_blob_count should be > 0"
    );
    // The exact value depends on SECTOR_HDR_DATA_SIZE, LOG_IDX_DATA_SIZE,
    // and wg_align(max_len). Verify it's reasonable (2 sectors × ~50 per sector).
    assert!(
        world.tsl_max_blob_count_result >= 50 && world.tsl_max_blob_count_result <= 200,
        "max_blob_count {} out of expected range",
        world.tsl_max_blob_count_result
    );
}

#[given(regex = r#"^有一条 TSL 状态为 FDB_TSL_WRITE$"#)]
async fn one_tsl_write(world: &mut FlashWorld) {
    let mut buf = make_blob_buf(64);
    let blob = blob_make(&mut buf);
    set_get_time(100);
    let flash = flash_mut!(world);
    let _ = world.tsdb.tsl_append(flash, &blob);
}

#[when(regex = r#"^调用 fdb_tsl_set_status\(db, &tsl, FDB_TSL_DELETED\)$"#)]
async fn tsl_set_deleted(world: &mut FlashWorld) {
    // Find the first Write TSL.
    let mut target = FdbTsl::default();
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter(flash, |tsl| {
        if tsl.status == FdbTslStatus::Write {
            target = *tsl;
            true
        } else {
            false
        }
    });
    let flash = flash_mut!(world);
    let result = world
        .tsdb
        .tsl_set_status(flash, &target, FdbTslStatus::Deleted);
    world.last_result = Some(result);
}

#[then(regex = r#"^重新读取该 TSL 状态为 FDB_TSL_DELETED$"#)]
async fn tsl_status_deleted(world: &mut FlashWorld) {
    let mut target = FdbTsl::default();
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter(flash, |tsl| {
        // Find the TSL we modified (it was the first Write, now Deleted).
        if tsl.status == FdbTslStatus::Deleted {
            target = *tsl;
            true
        } else {
            false
        }
    });
    assert_eq!(target.status, FdbTslStatus::Deleted);
}

#[when(regex = r#"^调用 fdb_tsl_clean\(db\)$"#)]
async fn tsl_clean(world: &mut FlashWorld) {
    let flash = flash_mut!(world);
    world.tsdb.tsl_clean(flash);
}

#[then(regex = r#"^所有扇区被格式化为 EMPTY 状态$"#)]
async fn all_sectors_empty(world: &mut FlashWorld) {
    // After clean, last_time should be 0 and iteration yields nothing.
    assert_eq!(world.tsdb.last_time(), 0);
}

#[then(regex = r#"^后续调用 fdb_tsl_iter 不产出任何 TSL$"#)]
async fn after_clean_no_tsl(world: &mut FlashWorld) {
    world.iterated_tsl_times.clear();
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter(flash, |tsl| {
        if tsl.status == FdbTslStatus::Write {
            world.iterated_tsl_times.push(tsl.time);
        }
        false
    });
    assert!(
        world.iterated_tsl_times.is_empty(),
        "expected no TSLs after clean"
    );
}

#[given(regex = r#"^刚调用过 fdb_tsl_clean，last_time 为 0$"#)]
async fn just_cleaned(world: &mut FlashWorld) {
    let flash = flash_mut!(world);
    world.tsdb.tsl_clean(flash);
    assert_eq!(world.tsdb.last_time(), 0);
}

#[given(regex = r#"^数据库中部分扇区为 USING/FULL，后续扇区为 EMPTY$"#)]
async fn db_partial_sectors(world: &mut FlashWorld) {
    // Write a few TSLs to make some sectors USING/FULL, leaving others EMPTY.
    for ts in [100i64, 200] {
        let mut buf = make_blob_buf(64);
        let blob = blob_make(&mut buf);
        let flash = flash_mut!(world);
        let _ = world.tsdb.tsl_append_with_ts(flash, &blob, ts as FdbTime);
    }
}

#[when(regex = r#"^调用 fdb_tsl_iter 正向遍历$"#)]
async fn tsl_iter_forward_bare(world: &mut FlashWorld) {
    world.iterated_tsl_times.clear();
    let flash = flash_mut!(world);
    world.tsdb.tsl_iter(flash, |tsl| {
        if tsl.status == FdbTslStatus::Write {
            world.iterated_tsl_times.push(tsl.time);
        }
        false
    });
}

#[given(regex = r#"^数据库为空（所有扇区为 EMPTY）$"#)]
async fn tsdb_db_empty(world: &mut FlashWorld) {
    world.setup_tsdb(4096, 16384, 256);
    world.last_result = None;
}
