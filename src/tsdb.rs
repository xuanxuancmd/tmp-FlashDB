// c: fdb_tsdb.c — TSDB (Time Series Database) implementation
//
// 1:1 Rust translation of fdb_tsdb.c (1118 lines).
// All flash I/O goes through the `FlashDevice` trait (see flash_trait.rs),
// replacing the C `db->storage` union dispatch. The `FdbTsdb` struct (defined
// in def.rs) does NOT own a flash handle; every method that performs I/O
// receives `&F` / `&mut F` as a separate parameter.

#![allow(dead_code)]

use crate::def::{
    FdbBlob, FdbDb, FdbErr, FdbSectorStoreStatus, FdbTime, FdbTsl, FdbTslStatus, FdbTsdb,
    TsdbSecInfo, FDB_BYTE_ERASED, FDB_DATA_UNUSED, FDB_FAILED_ADDR,
    FDB_SECTOR_STORE_STATUS_NUM, FDB_STORE_STATUS_TABLE_SIZE, FDB_TSL_STATUS_NUM,
};
use crate::flash_trait::FlashDevice;
use crate::low_lvl::{
    align_down, flash_erase, flash_read, flash_write, flash_write_align, get_status, read_status,
    set_status, status_table_size, wg_align, wg_align_down, write_status,
};

// ==========================================================================
// Constants (c: fdb_tsdb.c:28-99)
// ==========================================================================

/// c: fdb_tsdb.c:29 — magic word('T', 'S', 'L', '0')
const SECTOR_MAGIC_WORD: u32 = 0x304C_5354;

/// c: fdb_tsdb.c:31 — TSL_STATUS_TABLE_SIZE
const TSL_STATUS_TABLE_SIZE: usize = status_table_size(FDB_TSL_STATUS_NUM as u32) as usize;

/// c: fdb_tsdb.c:32 — TSL_UINT32_ALIGN_SIZE = FDB_WG_ALIGN(sizeof(uint32_t))
const TSL_UINT32_ALIGN_SIZE: usize = wg_align(4) as usize;

/// c: fdb_tsdb.c:34-38 — TSL_TIME_ALIGN_SIZE = FDB_WG_ALIGN(sizeof(fdb_time_t))
const TSL_TIME_ALIGN_SIZE: usize = wg_align(core::mem::size_of::<FdbTime>() as u32) as usize;

/// c: fdb_tsdb.c:41 — SECTOR_HDR_PADDING_SIZE = FDB_WG_ALIGN(4) - 4
const SECTOR_HDR_PADDING_SIZE: usize = (wg_align(4) - 4) as usize;

/// c: fdb_tsdb.c:44-48 — _TSL_FDBTIME_SIZE
const TSL_FDBTIME_SIZE: usize = core::mem::size_of::<FdbTime>();

/// c: fdb_tsdb.c:50-54 — LOG_IDX_BASE_SIZE (without FDB_TSDB_FIXED_BLOB_SIZE)
#[cfg(not(feature = "fixed_blob_size"))]
const LOG_IDX_BASE_SIZE: usize = TSL_STATUS_TABLE_SIZE + TSL_FDBTIME_SIZE + 4 * 2;

/// c: fdb_tsdb.c:51 — LOG_IDX_BASE_SIZE (with FDB_TSDB_FIXED_BLOB_SIZE)
#[cfg(feature = "fixed_blob_size")]
const LOG_IDX_BASE_SIZE: usize = TSL_STATUS_TABLE_SIZE + TSL_FDBTIME_SIZE;

/// c: fdb_tsdb.c:56 — LOG_IDX_PADDING_SIZE = FDB_WG_ALIGN(LOG_IDX_BASE_SIZE) - LOG_IDX_BASE_SIZE
const LOG_IDX_PADDING_SIZE: usize = (wg_align(LOG_IDX_BASE_SIZE as u32) as usize) - LOG_IDX_BASE_SIZE;

/// c: fdb_cfg_template.h:30 — FDB_TSDB_FIXED_BLOB_SIZE
///
/// When the `fixed_blob_size` Cargo feature is enabled, all TSL blobs have this
/// fixed size. The default value of 4 matches the C template comment
/// (`/* #define FDB_TSDB_FIXED_BLOB_SIZE 4 */`).
#[cfg(feature = "fixed_blob_size")]
pub const FDB_TSDB_FIXED_BLOB_SIZE: usize = 4;

/// c: fdb_tsdb.c:71 — FAILED_ADDR (the next address is get failed)
///
/// Already defined as `FDB_FAILED_ADDR` in def.rs; re-exported here for
/// readability within this module.
const FAILED_ADDR: u32 = FDB_FAILED_ADDR;

// ---------------------------------------------------------------------------
// Helper: align_up for usize (const fn, used in layout computations below)
// ---------------------------------------------------------------------------

const fn align_up_usize(size: usize, align: usize) -> usize {
    if align == 0 {
        return size;
    }
    (size + align - 1) / align * align
}

// ==========================================================================
// On-flash structs (c: fdb_tsdb.c:101-133)
// ==========================================================================

/// c: fdb_tsdb.c:105-109 — end_info entry inside sector_hdr_data
///
/// Each sector header stores two end-info entries (a double-buffer scheme for
/// crash-safe sector closing). All fields are byte arrays to match the C
/// `uint8_t[]` layout (alignment 1, no implicit padding within the entry).
#[repr(C)]
#[derive(Clone, Copy)]
struct TsdbSectorEndInfo {
    /// c: fdb_tsdb.c:106 — the last end node's timestamp
    time: [u8; TSL_TIME_ALIGN_SIZE],
    /// c: fdb_tsdb.c:107 — the last end node's index
    index: [u8; TSL_UINT32_ALIGN_SIZE],
    /// c: fdb_tsdb.c:108 — end node status
    status: [u8; TSL_STATUS_TABLE_SIZE],
}

impl Default for TsdbSectorEndInfo {
    fn default() -> Self {
        Self {
            time: [FDB_BYTE_ERASED; TSL_TIME_ALIGN_SIZE],
            index: [FDB_BYTE_ERASED; TSL_UINT32_ALIGN_SIZE],
            status: [FDB_BYTE_ERASED; TSL_STATUS_TABLE_SIZE],
        }
    }
}

/// c: fdb_tsdb.c:101-116 — struct sector_hdr_data (on-flash)
///
/// The sector header stored at the beginning of each flash sector. All time /
/// index / status fields are byte arrays matching the C layout. The `reserved`
/// field is `uint32_t` (alignment 4) which introduces padding before it.
#[repr(C)]
#[derive(Clone, Copy)]
struct TsdbSectorHdrData {
    /// c: fdb_tsdb.c:102 — sector store status
    status: [u8; FDB_STORE_STATUS_TABLE_SIZE],
    /// c: fdb_tsdb.c:103 — magic word('T', 'S', 'L', '0')
    magic: [u8; TSL_UINT32_ALIGN_SIZE],
    /// c: fdb_tsdb.c:104 — the first start node's timestamp
    start_time: [u8; TSL_TIME_ALIGN_SIZE],
    /// c: fdb_tsdb.c:105-109 — end_info[2]
    end_info: [TsdbSectorEndInfo; 2],
    /// c: fdb_tsdb.c:110 — reserved
    reserved: u32,
    /// c: fdb_tsdb.c:113-115 — padding to FDB WRITE GRAN alignment
    padding: [u8; SECTOR_HDR_PADDING_SIZE],
}

impl Default for TsdbSectorHdrData {
    fn default() -> Self {
        Self {
            status: [FDB_BYTE_ERASED; FDB_STORE_STATUS_TABLE_SIZE],
            magic: [FDB_BYTE_ERASED; TSL_UINT32_ALIGN_SIZE],
            start_time: [FDB_BYTE_ERASED; TSL_TIME_ALIGN_SIZE],
            end_info: [TsdbSectorEndInfo::default(); 2],
            reserved: u32::MAX,
            padding: [FDB_BYTE_ERASED; SECTOR_HDR_PADDING_SIZE],
        }
    }
}

/// c: fdb_tsdb.c:120-132 — struct log_idx_data (on-flash)
///
/// Time series log node index data. The `time` field uses `FdbTime` (i32 or
/// i64 depending on the `timestamp_64bit` feature) matching the C
/// `fdb_time_t` type. When `FDB_TSDB_FIXED_BLOB_SIZE` is enabled the
/// `log_len` / `log_addr` fields are absent (the blob size and address are
/// computed from the index position instead).
#[repr(C)]
#[derive(Clone, Copy)]
struct LogIdxData {
    /// c: fdb_tsdb.c:121 — node status
    status_table: [u8; TSL_STATUS_TABLE_SIZE],
    /// c: fdb_tsdb.c:122 — node timestamp
    time: FdbTime,
    /// c: fdb_tsdb.c:124 — node total length (header + name + value)
    #[cfg(not(feature = "fixed_blob_size"))]
    log_len: u32,
    /// c: fdb_tsdb.c:125 — node address
    #[cfg(not(feature = "fixed_blob_size"))]
    log_addr: u32,
    /// c: fdb_tsdb.c:129-131 — padding to FDB WRITE GRAN alignment
    padding: [u8; LOG_IDX_PADDING_SIZE],
}

impl Default for LogIdxData {
    fn default() -> Self {
        Self {
            status_table: [FDB_BYTE_ERASED; TSL_STATUS_TABLE_SIZE],
            time: 0,
            #[cfg(not(feature = "fixed_blob_size"))]
            log_len: u32::MAX,
            #[cfg(not(feature = "fixed_blob_size"))]
            log_addr: u32::MAX,
            padding: [FDB_BYTE_ERASED; LOG_IDX_PADDING_SIZE],
        }
    }
}

// ==========================================================================
// Layout assertions (c: fdb_tsdb.c:58-68)
// ==========================================================================

/// c: fdb_tsdb.c:58 — SECTOR_HDR_DATA_SIZE = FDB_WG_ALIGN(sizeof(struct sector_hdr_data))
const SECTOR_HDR_DATA_SIZE: usize = wg_align(core::mem::size_of::<TsdbSectorHdrData>() as u32) as usize;

/// c: fdb_tsdb.c:59 — LOG_IDX_DATA_SIZE = FDB_WG_ALIGN(sizeof(struct log_idx_data))
const LOG_IDX_DATA_SIZE: usize = wg_align(core::mem::size_of::<LogIdxData>() as u32) as usize;

// --- Offset constants (c: fdb_tsdb.c:60-68) ---

/// c: fdb_tsdb.c:60 — LOG_IDX_TS_OFFSET
const LOG_IDX_TS_OFFSET: usize = core::mem::offset_of!(LogIdxData, time);

/// c: fdb_tsdb.c:61 — SECTOR_MAGIC_OFFSET
const SECTOR_MAGIC_OFFSET: usize = core::mem::offset_of!(TsdbSectorHdrData, magic);

/// c: fdb_tsdb.c:62 — SECTOR_START_TIME_OFFSET
const SECTOR_START_TIME_OFFSET: usize = core::mem::offset_of!(TsdbSectorHdrData, start_time);

/// Offset of the `end_info` array within `TsdbSectorHdrData`.
const SECTOR_END_INFO_OFFSET: usize = core::mem::offset_of!(TsdbSectorHdrData, end_info);

/// Size of a single `TsdbSectorEndInfo` entry.
const END_INFO_SIZE: usize = core::mem::size_of::<TsdbSectorEndInfo>();

/// Offset of `time` within `TsdbSectorEndInfo`.
const END_INFO_TIME_OFF: usize = core::mem::offset_of!(TsdbSectorEndInfo, time);

/// Offset of `index` within `TsdbSectorEndInfo`.
const END_INFO_IDX_OFF: usize = core::mem::offset_of!(TsdbSectorEndInfo, index);

/// Offset of `status` within `TsdbSectorEndInfo`.
const END_INFO_STATUS_OFF: usize = core::mem::offset_of!(TsdbSectorEndInfo, status);

/// c: fdb_tsdb.c:63 — SECTOR_END0_TIME_OFFSET
const SECTOR_END0_TIME_OFFSET: usize = SECTOR_END_INFO_OFFSET + 0 * END_INFO_SIZE + END_INFO_TIME_OFF;

/// c: fdb_tsdb.c:64 — SECTOR_END0_IDX_OFFSET
const SECTOR_END0_IDX_OFFSET: usize = SECTOR_END_INFO_OFFSET + 0 * END_INFO_SIZE + END_INFO_IDX_OFF;

/// c: fdb_tsdb.c:65 — SECTOR_END0_STATUS_OFFSET
const SECTOR_END0_STATUS_OFFSET: usize =
    SECTOR_END_INFO_OFFSET + 0 * END_INFO_SIZE + END_INFO_STATUS_OFF;

/// c: fdb_tsdb.c:66 — SECTOR_END1_TIME_OFFSET
const SECTOR_END1_TIME_OFFSET: usize = SECTOR_END_INFO_OFFSET + 1 * END_INFO_SIZE + END_INFO_TIME_OFF;

/// c: fdb_tsdb.c:67 — SECTOR_END1_IDX_OFFSET
const SECTOR_END1_IDX_OFFSET: usize = SECTOR_END_INFO_OFFSET + 1 * END_INFO_SIZE + END_INFO_IDX_OFF;

/// c: fdb_tsdb.c:68 — SECTOR_END1_STATUS_OFFSET
const SECTOR_END1_STATUS_OFFSET: usize =
    SECTOR_END_INFO_OFFSET + 1 * END_INFO_SIZE + END_INFO_STATUS_OFF;

// --- Compile-time size verification ---
//
// The expected sizes are computed from the field sizes and C alignment rules,
// independent of `offset_of!` / `size_of!`, so that the assertion truly
// cross-checks the Rust layout against the C layout.

/// Expected size of `TsdbSectorEndInfo` (all u8 arrays, align 1, no padding).
const EXPECTED_END_INFO_SIZE: usize = TSL_TIME_ALIGN_SIZE + TSL_UINT32_ALIGN_SIZE + TSL_STATUS_TABLE_SIZE;

/// Expected size of `TsdbSectorHdrData`.
const EXPECTED_SECTOR_HDR_SIZE: usize = {
    // Bytes before `reserved` (all u8 arrays + end_info array, align 1):
    let before_reserved = FDB_STORE_STATUS_TABLE_SIZE
        + TSL_UINT32_ALIGN_SIZE
        + TSL_TIME_ALIGN_SIZE
        + 2 * EXPECTED_END_INFO_SIZE;
    // `reserved: u32` requires 4-byte alignment:
    let reserved_offset = align_up_usize(before_reserved, 4);
    let after_reserved = reserved_offset + 4 + SECTOR_HDR_PADDING_SIZE;
    // Struct alignment is 4 (from u32); round up:
    align_up_usize(after_reserved, 4)
};

/// Expected size of `LogIdxData` (variable-size blob mode).
#[cfg(not(feature = "fixed_blob_size"))]
const EXPECTED_LOG_IDX_SIZE: usize = {
    let time_align = core::mem::align_of::<FdbTime>();
    let time_offset = align_up_usize(TSL_STATUS_TABLE_SIZE, time_align);
    let after_time = time_offset + TSL_FDBTIME_SIZE;
    // log_len (u32) + log_addr (u32), both align 4, already aligned after time:
    let after_log_addr = after_time + 4 + 4;
    let total = after_log_addr + LOG_IDX_PADDING_SIZE;
    // Struct alignment = max(time_align, 4) = time_align (>= 4):
    align_up_usize(total, time_align)
};

/// Expected size of `LogIdxData` (fixed-size blob mode).
#[cfg(feature = "fixed_blob_size")]
const EXPECTED_LOG_IDX_SIZE: usize = {
    let time_align = core::mem::align_of::<FdbTime>();
    let time_offset = align_up_usize(TSL_STATUS_TABLE_SIZE, time_align);
    let after_time = time_offset + TSL_FDBTIME_SIZE;
    let total = after_time + LOG_IDX_PADDING_SIZE;
    align_up_usize(total, time_align)
};

// Compile-time layout assertions — these will fail the build if the Rust
// `#[repr(C)]` layout does not match the C layout.
const _: () = assert!(core::mem::size_of::<TsdbSectorEndInfo>() == EXPECTED_END_INFO_SIZE);
const _: () = assert!(core::mem::size_of::<TsdbSectorHdrData>() == EXPECTED_SECTOR_HDR_SIZE);
const _: () = assert!(core::mem::size_of::<LogIdxData>() == EXPECTED_LOG_IDX_SIZE);
// Verify the aligned sizes used for flash I/O:
const _: () = assert!(SECTOR_HDR_DATA_SIZE == wg_align(EXPECTED_SECTOR_HDR_SIZE as u32) as usize);
const _: () = assert!(LOG_IDX_DATA_SIZE == wg_align(EXPECTED_LOG_IDX_SIZE as u32) as usize);

// ==========================================================================
// Status / time helper functions
// ==========================================================================

/// Convert a status index to `FdbSectorStoreStatus`.
fn sector_store_status_from_index(index: usize) -> FdbSectorStoreStatus {
    match index {
        0 => FdbSectorStoreStatus::Unused,
        1 => FdbSectorStoreStatus::Empty,
        2 => FdbSectorStoreStatus::Using,
        3 => FdbSectorStoreStatus::Full,
        _ => FdbSectorStoreStatus::Unused,
    }
}

/// Convert a status index to `FdbTslStatus`.
fn tsl_status_from_index(index: usize) -> FdbTslStatus {
    match index {
        0 => FdbTslStatus::Unused,
        1 => FdbTslStatus::PreWrite,
        2 => FdbTslStatus::Write,
        3 => FdbTslStatus::UserStatus1,
        4 => FdbTslStatus::Deleted,
        5 => FdbTslStatus::UserStatus2,
        _ => FdbTslStatus::Unused,
    }
}

/// Read a native-endian `FdbTime` from a byte buffer at `offset`.
fn read_time_ne(buf: &[u8], offset: usize) -> FdbTime {
    let len = TSL_FDBTIME_SIZE;
    #[cfg(not(feature = "timestamp_64bit"))]
    {
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&buf[offset..offset + len]);
        i32::from_ne_bytes(arr) as FdbTime
    }
    #[cfg(feature = "timestamp_64bit")]
    {
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&buf[offset..offset + len]);
        i64::from_ne_bytes(arr) as FdbTime
    }
}

/// Write a native-endian `FdbTime` into a byte buffer at `offset`.
fn write_time_ne(buf: &mut [u8], offset: usize, val: FdbTime) {
    let len = TSL_FDBTIME_SIZE;
    #[cfg(not(feature = "timestamp_64bit"))]
    {
        buf[offset..offset + len].copy_from_slice(&(val as i32).to_ne_bytes());
    }
    #[cfg(feature = "timestamp_64bit")]
    {
        buf[offset..offset + len].copy_from_slice(&(val as i64).to_ne_bytes());
    }
}

/// Read a native-endian `u32` from a byte buffer at `offset`.
fn read_u32_ne(buf: &[u8], offset: usize) -> u32 {
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&buf[offset..offset + 4]);
    u32::from_ne_bytes(arr)
}

/// Write a native-endian `u32` into a byte buffer at `offset`.
fn write_u32_ne(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_ne_bytes());
}

// ==========================================================================
// FdbTsdb implementation
// ==========================================================================

impl FdbTsdb {
    // ===== T15: Core read/write functions =====
    // (Implemented in T15)

    // ===== T16: Init / deinit / control / clean =====
    // (Implemented in T16)

    // ===== T17: Iter / query / set_status =====
    // (Implemented in T17)
}

// ==========================================================================
// Unit tests
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::def::{
        FdbSectorStoreStatus, FdbTslStatus, FDB_SECTOR_STORE_STATUS_NUM, FDB_STORE_STATUS_TABLE_SIZE,
        FDB_TSL_STATUS_NUM,
    };
    use crate::low_lvl::{status_table_size, wg_align};
    use crate::mock_flash::MockFlash;

    // ---- T14: Layout verification tests ----

    #[test]
    fn test_tsdb_sector_hdr_size() {
        // The struct size must match the expected C layout.
        assert_eq!(
            core::mem::size_of::<TsdbSectorHdrData>(),
            EXPECTED_SECTOR_HDR_SIZE,
            "TsdbSectorHdrData size must match C layout"
        );
    }

    #[test]
    fn test_log_idx_size() {
        assert_eq!(
            core::mem::size_of::<LogIdxData>(),
            EXPECTED_LOG_IDX_SIZE,
            "LogIdxData size must match C layout"
        );
    }

    #[test]
    fn test_end_info_size() {
        assert_eq!(
            core::mem::size_of::<TsdbSectorEndInfo>(),
            EXPECTED_END_INFO_SIZE,
            "TsdbSectorEndInfo size must match C layout"
        );
    }

    #[test]
    fn test_tsdb_offsets() {
        // SECTOR_MAGIC_OFFSET — right after status[FDB_STORE_STATUS_TABLE_SIZE]
        assert_eq!(
            SECTOR_MAGIC_OFFSET,
            FDB_STORE_STATUS_TABLE_SIZE,
            "SECTOR_MAGIC_OFFSET must be right after status field"
        );

        // SECTOR_START_TIME_OFFSET — after status + magic
        assert_eq!(
            SECTOR_START_TIME_OFFSET,
            FDB_STORE_STATUS_TABLE_SIZE + TSL_UINT32_ALIGN_SIZE,
            "SECTOR_START_TIME_OFFSET must be after status + magic"
        );

        // LOG_IDX_TS_OFFSET — after status_table, aligned to FdbTime
        let expected_ts_offset = align_up_usize(TSL_STATUS_TABLE_SIZE, core::mem::align_of::<FdbTime>());
        assert_eq!(
            LOG_IDX_TS_OFFSET,
            expected_ts_offset,
            "LOG_IDX_TS_OFFSET must be aligned to FdbTime"
        );

        // End-info offsets are contiguous within the end_info array
        assert_eq!(SECTOR_END0_TIME_OFFSET, SECTOR_END_INFO_OFFSET + END_INFO_TIME_OFF);
        assert_eq!(SECTOR_END0_IDX_OFFSET, SECTOR_END_INFO_OFFSET + END_INFO_IDX_OFF);
        assert_eq!(SECTOR_END0_STATUS_OFFSET, SECTOR_END_INFO_OFFSET + END_INFO_STATUS_OFF);

        assert_eq!(
            SECTOR_END1_TIME_OFFSET,
            SECTOR_END_INFO_OFFSET + END_INFO_SIZE + END_INFO_TIME_OFF
        );
        assert_eq!(
            SECTOR_END1_IDX_OFFSET,
            SECTOR_END_INFO_OFFSET + END_INFO_SIZE + END_INFO_IDX_OFF
        );
        assert_eq!(
            SECTOR_END1_STATUS_OFFSET,
            SECTOR_END_INFO_OFFSET + END_INFO_SIZE + END_INFO_STATUS_OFF
        );
    }

    #[test]
    fn test_header_layouts() {
        // Combined layout verification: sizes, offsets, and aligned sizes
        // must all be self-consistent and match the C definitions.

        // 1. Struct sizes match expected
        assert_eq!(core::mem::size_of::<TsdbSectorHdrData>(), EXPECTED_SECTOR_HDR_SIZE);
        assert_eq!(core::mem::size_of::<LogIdxData>(), EXPECTED_LOG_IDX_SIZE);
        assert_eq!(core::mem::size_of::<TsdbSectorEndInfo>(), EXPECTED_END_INFO_SIZE);

        // 2. Aligned sizes used for flash I/O
        assert_eq!(SECTOR_HDR_DATA_SIZE, wg_align(EXPECTED_SECTOR_HDR_SIZE as u32) as usize);
        assert_eq!(LOG_IDX_DATA_SIZE, wg_align(EXPECTED_LOG_IDX_SIZE as u32) as usize);

        // 3. SECTOR_HDR_DATA_SIZE must be a multiple of the write granularity
        let wg_bytes = wg_align(1) as usize;
        assert_eq!(SECTOR_HDR_DATA_SIZE % wg_bytes, 0, "SECTOR_HDR_DATA_SIZE must be WG-aligned");
        assert_eq!(LOG_IDX_DATA_SIZE % wg_bytes, 0, "LOG_IDX_DATA_SIZE must be WG-aligned");

        // 4. Magic word value
        assert_eq!(SECTOR_MAGIC_WORD, 0x304C_5354, "magic word must be 'T','S','L','0'");

        // 5. FAILED_ADDR
        assert_eq!(FAILED_ADDR, 0xFFFF_FFFF);
    }

    #[test]
    fn test_default_structs_are_erased() {
        // Default structs should have erased (0xFF) byte-array fields,
        // matching a freshly-erased flash sector.
        let hdr = TsdbSectorHdrData::default();
        for &b in &hdr.status {
            assert_eq!(b, FDB_BYTE_ERASED);
        }
        for &b in &hdr.magic {
            assert_eq!(b, FDB_BYTE_ERASED);
        }
        for &b in &hdr.start_time {
            assert_eq!(b, FDB_BYTE_ERASED);
        }
        for ei in &hdr.end_info {
            for &b in &ei.time {
                assert_eq!(b, FDB_BYTE_ERASED);
            }
            for &b in &ei.index {
                assert_eq!(b, FDB_BYTE_ERASED);
            }
            for &b in &ei.status {
                assert_eq!(b, FDB_BYTE_ERASED);
            }
        }

        let idx = LogIdxData::default();
        for &b in &idx.status_table {
            assert_eq!(b, FDB_BYTE_ERASED);
        }
    }

    #[test]
    fn test_status_table_sizes() {
        // TSL_STATUS_TABLE_SIZE must equal FDB_STATUS_TABLE_SIZE(FDB_TSL_STATUS_NUM)
        assert_eq!(
            TSL_STATUS_TABLE_SIZE,
            status_table_size(FDB_TSL_STATUS_NUM as u32) as usize
        );
        // FDB_STORE_STATUS_TABLE_SIZE is defined in def.rs
        assert_eq!(
            FDB_STORE_STATUS_TABLE_SIZE,
            status_table_size(FDB_SECTOR_STORE_STATUS_NUM as u32) as usize
        );
    }

    #[test]
    fn test_status_conversion_roundtrip() {
        // Sector store status roundtrip
        for i in 0..FDB_SECTOR_STORE_STATUS_NUM as usize {
            let status = sector_store_status_from_index(i);
            assert_eq!(status as usize, i, "sector store status index must roundtrip");
        }

        // TSL status roundtrip
        for i in 0..FDB_TSL_STATUS_NUM as usize {
            let status = tsl_status_from_index(i);
            assert_eq!(status as usize, i, "TSL status index must roundtrip");
        }
    }

    #[test]
    fn test_time_read_write_ne() {
        // Write and read back a time value in native endianness
        let mut buf = [0u8; 32];
        let val: FdbTime = 123456;
        write_time_ne(&mut buf, 4, val);
        assert_eq!(read_time_ne(&buf, 4), val);

        // Negative timestamp
        let neg: FdbTime = -1;
        write_time_ne(&mut buf, 0, neg);
        assert_eq!(read_time_ne(&buf, 0), neg);
    }

    #[test]
    fn test_u32_read_write_ne() {
        let mut buf = [0u8; 16];
        write_u32_ne(&mut buf, 0, 0xDEAD_BEEF);
        assert_eq!(read_u32_ne(&buf, 0), 0xDEAD_BEEF);

        write_u32_ne(&mut buf, 4, SECTOR_MAGIC_WORD);
        assert_eq!(read_u32_ne(&buf, 4), SECTOR_MAGIC_WORD);
    }
}
