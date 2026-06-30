// c: fdb_tsdb.c — TSDB (Time Series Database) implementation
//
// 1:1 Rust translation of fdb_tsdb.c (1118 lines).
// All flash I/O goes through the `FlashDevice` trait (see flash_trait.rs),
// replacing the C `db->storage` union dispatch. The `FdbTsdb` struct (defined
// in def.rs) does NOT own a flash handle; every method that performs I/O
// receives `&F` / `&mut F` as a separate parameter.

#![allow(dead_code)]

use crate::def::{
    FdbBlob, FdbDb, FdbDbType, FdbErr, FdbGetTime, FdbSectorStoreStatus, FdbTime, FdbTsl,
    FdbTslStatus, FdbTsdb, TsdbSecInfo, FDB_BYTE_ERASED, FDB_DATA_UNUSED, FDB_FAILED_ADDR,
    FDB_SECTOR_STORE_STATUS_NUM, FDB_STORE_STATUS_TABLE_SIZE, FDB_TSL_STATUS_NUM,
};
use crate::flash_trait::FlashDevice;
use crate::init::{init_ex, init_finish, deinit as db_deinit};
use crate::low_lvl::{
    align_down, align_up, flash_erase, flash_read, flash_write, flash_write_align, get_status,
    status_table_size, wg_align, write_status,
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
    // ===== Helper: db_lock / db_unlock (c: fdb_tsdb.c:79-87) =====

    /// c: fdb_tsdb.c:79-82 — db_lock macro
    fn db_lock(&mut self) {
        let lock = self.parent.lock;
        if let Some(lock_fn) = lock {
            lock_fn(&mut self.parent);
        }
    }

    /// c: fdb_tsdb.c:84-87 — db_unlock macro
    fn db_unlock(&mut self) {
        let unlock = self.parent.unlock;
        if let Some(unlock_fn) = unlock {
            unlock_fn(&mut self.parent);
        }
    }

    // ===== T15: Core read/write functions (c: fdb_tsdb.c:147-547) =====

    /// c: fdb_tsdb.c:147-175 — read_tsl
    ///
    /// Read a TSL node's index data from flash and populate the `tsl` struct.
    /// The C version ignores the `_fdb_flash_read` return value; this
    /// translation matches that behaviour.
    fn read_tsl<F: FlashDevice>(&self, flash: &F, tsl: &mut FdbTsl) {
        // c: fdb_tsdb.c:149 — read TSL index raw data
        let mut buf = [0u8; core::mem::size_of::<LogIdxData>()];
        let _ = flash_read(flash, tsl.addr_index, &mut buf);

        // c: fdb_tsdb.c:153 — tsl->status = _fdb_get_status(idx.status_table, FDB_TSL_STATUS_NUM)
        let status_idx = get_status(&buf[..TSL_STATUS_TABLE_SIZE], FDB_TSL_STATUS_NUM as usize);
        tsl.status = tsl_status_from_index(status_idx);

        if tsl.status == FdbTslStatus::PreWrite || tsl.status == FdbTslStatus::Unused {
            // c: fdb_tsdb.c:154-157
            tsl.log_len = self.max_len as u32;
            tsl.addr_log = FDB_DATA_UNUSED;
            tsl.time = 0;
        } else {
            // c: fdb_tsdb.c:159-171
            #[cfg(feature = "fixed_blob_size")]
            {
                // c: fdb_tsdb.c:160-165
                let sector_addr = align_down(tsl.addr_index, self.parent.sec_size);
                let tsl_index_in_sector =
                    (tsl.addr_index - sector_addr - SECTOR_HDR_DATA_SIZE as u32)
                        / LOG_IDX_DATA_SIZE as u32;
                tsl.log_len = FDB_TSDB_FIXED_BLOB_SIZE as u32;
                tsl.addr_log = sector_addr
                    + self.parent.sec_size
                    - (tsl_index_in_sector + 1) * wg_align(FDB_TSDB_FIXED_BLOB_SIZE as u32);
                tsl.time = read_time_ne(&buf, LOG_IDX_TS_OFFSET);
            }
            #[cfg(not(feature = "fixed_blob_size"))]
            {
                // c: fdb_tsdb.c:168-170
                tsl.log_len = read_u32_ne(&buf, LOG_IDX_TS_OFFSET + TSL_FDBTIME_SIZE);
                tsl.addr_log = read_u32_ne(&buf, LOG_IDX_TS_OFFSET + TSL_FDBTIME_SIZE + 4);
                tsl.time = read_time_ne(&buf, LOG_IDX_TS_OFFSET);
            }
        }
    }

    /// c: fdb_tsdb.c:177-190 — get_next_sector_addr
    fn get_next_sector_addr(&self, pre_sec: &TsdbSecInfo, traversed_len: u32) -> u32 {
        if traversed_len + self.parent.sec_size <= self.parent.max_size {
            if pre_sec.addr + self.parent.sec_size < self.parent.max_size {
                // c: fdb_tsdb.c:181
                pre_sec.addr + self.parent.sec_size
            } else {
                // c: fdb_tsdb.c:183 — the next sector is on the top of the database
                0
            }
        } else {
            // c: fdb_tsdb.c:187 — finished
            FAILED_ADDR
        }
    }

    /// c: fdb_tsdb.c:192-208 — get_next_tsl_addr
    fn get_next_tsl_addr(sector: &TsdbSecInfo, pre_tsl: &FdbTsl) -> u32 {
        if sector.status == FdbSectorStoreStatus::Empty {
            return FAILED_ADDR;
        }
        if pre_tsl.addr_index + LOG_IDX_DATA_SIZE as u32 <= sector.end_idx {
            pre_tsl.addr_index + LOG_IDX_DATA_SIZE as u32
        } else {
            FAILED_ADDR
        }
    }

    /// c: fdb_tsdb.c:210-225 — get_last_tsl_addr
    fn get_last_tsl_addr(sector: &TsdbSecInfo, pre_tsl: &FdbTsl) -> u32 {
        if sector.status == FdbSectorStoreStatus::Empty {
            return FAILED_ADDR;
        }
        if pre_tsl.addr_index >= (sector.addr + SECTOR_HDR_DATA_SIZE as u32 + LOG_IDX_DATA_SIZE as u32)
        {
            pre_tsl.addr_index - LOG_IDX_DATA_SIZE as u32
        } else {
            FAILED_ADDR
        }
    }

    /// c: fdb_tsdb.c:227-240 — get_last_sector_addr
    fn get_last_sector_addr(&self, pre_sec: &TsdbSecInfo, traversed_len: u32) -> u32 {
        if traversed_len + self.parent.sec_size <= self.parent.max_size {
            if pre_sec.addr >= self.parent.sec_size {
                // c: fdb_tsdb.c:231 — the next sector is previous sector
                pre_sec.addr - self.parent.sec_size
            } else {
                // c: fdb_tsdb.c:233 — the next sector is the last sector
                self.parent.max_size - self.parent.sec_size
            }
        } else {
            FAILED_ADDR
        }
    }

    /// c: fdb_tsdb.c:242-307 — read_sector_info
    ///
    /// Read sector header from flash and populate the `sector` struct. When
    /// `traversal` is true and the sector is USING, iterate all TSLs to
    /// compute the actual empty space and end position.
    fn read_sector_info<F: FlashDevice>(
        &self,
        flash: &F,
        addr: u32,
        sector: &mut TsdbSecInfo,
        traversal: bool,
    ) -> Result<(), FdbErr> {
        // c: fdb_tsdb.c:245 — read sector header raw data
        let mut buf = [0u8; core::mem::size_of::<TsdbSectorHdrData>()];
        let _ = flash_read(flash, addr, &mut buf);

        // c: fdb_tsdb.c:252-253
        sector.addr = addr;
        sector.magic = read_u32_ne(&buf, SECTOR_MAGIC_OFFSET);

        // c: fdb_tsdb.c:255-259 — check magic word
        if sector.magic != SECTOR_MAGIC_WORD {
            sector.check_ok = false;
            return Err(FdbErr::InitFailed);
        }
        sector.check_ok = true;

        // c: fdb_tsdb.c:261 — sector->status = _fdb_get_status(sec_hdr.status, ...)
        let status_idx = get_status(
            &buf[..FDB_STORE_STATUS_TABLE_SIZE],
            FDB_SECTOR_STORE_STATUS_NUM as usize,
        );
        sector.status = sector_store_status_from_index(status_idx);

        // c: fdb_tsdb.c:262 — memcpy(&sector->start_time, sec_hdr.start_time, sizeof(fdb_time_t))
        sector.start_time = read_time_ne(&buf, SECTOR_START_TIME_OFFSET);

        // c: fdb_tsdb.c:263-264 — end_info_stat[0..1]
        let end0_status_idx = get_status(
            &buf[SECTOR_END0_STATUS_OFFSET..SECTOR_END0_STATUS_OFFSET + TSL_STATUS_TABLE_SIZE],
            FDB_TSL_STATUS_NUM as usize,
        );
        sector.end_info_stat[0] = tsl_status_from_index(end0_status_idx);

        let end1_status_idx = get_status(
            &buf[SECTOR_END1_STATUS_OFFSET..SECTOR_END1_STATUS_OFFSET + TSL_STATUS_TABLE_SIZE],
            FDB_TSL_STATUS_NUM as usize,
        );
        sector.end_info_stat[1] = tsl_status_from_index(end1_status_idx);

        // c: fdb_tsdb.c:265-274 — parse end_time and end_idx
        if sector.end_info_stat[0] == FdbTslStatus::Write {
            sector.end_time = read_time_ne(&buf, SECTOR_END0_TIME_OFFSET);
            sector.end_idx = read_u32_ne(&buf, SECTOR_END0_IDX_OFFSET);
        } else if sector.end_info_stat[1] == FdbTslStatus::Write {
            sector.end_time = read_time_ne(&buf, SECTOR_END1_TIME_OFFSET);
            sector.end_idx = read_u32_ne(&buf, SECTOR_END1_IDX_OFFSET);
        } else if sector.end_info_stat[0] == FdbTslStatus::PreWrite
            && sector.end_info_stat[1] == FdbTslStatus::PreWrite
        {
            // c: fdb_tsdb.c:273 — FDB_ASSERT(0)
            panic!(
                "read_sector_info: both end_info entries are PRE_WRITE (sector 0x{:08X})",
                addr
            );
        }

        // c: fdb_tsdb.c:276-279 — calculate remain space
        sector.empty_idx = sector.addr + SECTOR_HDR_DATA_SIZE as u32;
        sector.empty_data = sector.addr + self.parent.sec_size;
        sector.remain = (sector.empty_data - sector.empty_idx) as usize;

        // c: fdb_tsdb.c:280-304 — traversal all TSL
        if sector.status == FdbSectorStoreStatus::Using && traversal {
            let mut tsl = FdbTsl::default();
            tsl.addr_index = sector.empty_idx;
            loop {
                self.read_tsl(flash, &mut tsl);
                if tsl.status == FdbTslStatus::Unused {
                    break;
                }
                if tsl.status != FdbTslStatus::PreWrite {
                    sector.end_time = tsl.time;
                }
                sector.end_idx = tsl.addr_index;
                sector.empty_idx += LOG_IDX_DATA_SIZE as u32;
                sector.empty_data -= wg_align(tsl.log_len);
                tsl.addr_index += LOG_IDX_DATA_SIZE as u32;

                let tsl_size = LOG_IDX_DATA_SIZE as u32 + wg_align(tsl.log_len);
                if (sector.remain as u32) > tsl_size {
                    sector.remain -= tsl_size as usize;
                } else {
                    // c: fdb_tsdb.c:298-301 — TSL size out of bound
                    sector.remain = 0;
                    return Err(FdbErr::ReadErr);
                }
            }
        }

        Ok(())
    }

    /// c: fdb_tsdb.c:309-327 — format_sector
    ///
    /// Erase a sector and write the sector header (status=EMPTY + magic word).
    fn format_sector<F: FlashDevice>(&self, flash: &mut F, addr: u32) -> Result<(), FdbErr> {
        // c: fdb_tsdb.c:315 — FDB_ASSERT(addr % db_sec_size(db) == 0)
        assert!(
            addr % self.parent.sec_size == 0,
            "format_sector: addr must be sector-aligned"
        );

        // c: fdb_tsdb.c:317 — _fdb_flash_erase
        flash_erase(flash, addr, self.parent.sec_size)?;

        // c: fdb_tsdb.c:319 — _FDB_WRITE_STATUS(... FDB_SECTOR_STORE_EMPTY ...)
        let mut status_table = [0u8; FDB_STORE_STATUS_TABLE_SIZE];
        write_status(
            flash,
            addr,
            &mut status_table,
            FDB_SECTOR_STORE_STATUS_NUM as usize,
            FdbSectorStoreStatus::Empty as usize,
        )?;

        // c: fdb_tsdb.c:321-323 — set the magic word
        let mut magic_buf = [FDB_BYTE_ERASED; TSL_UINT32_ALIGN_SIZE];
        let magic_bytes = SECTOR_MAGIC_WORD.to_ne_bytes();
        let copy_len = 4.min(TSL_UINT32_ALIGN_SIZE);
        magic_buf[..copy_len].copy_from_slice(&magic_bytes[..copy_len]);
        flash_write(flash, addr + SECTOR_MAGIC_OFFSET as u32, &magic_buf)?;

        Ok(())
    }

    /// c: fdb_tsdb.c:350-377 — write_tsl
    ///
    /// Write a TSL node to flash using a two-phase status transition
    /// (PRE_WRITE → WRITE). The index is written at `cur_sec.empty_idx` and
    /// the blob data grows from the sector bottom (`cur_sec.empty_data`).
    fn write_tsl<F: FlashDevice>(
        &self,
        flash: &mut F,
        blob: &FdbBlob,
        time: FdbTime,
    ) -> Result<(), FdbErr> {
        // c: fdb_tsdb.c:354-355
        let idx_addr = self.cur_sec.empty_idx;
        let log_addr = self.cur_sec.empty_data - wg_align(blob.size as u32);

        // c: fdb_tsdb.c:357-361 — build index data (variable-size mode stores addr+len)
        let mut idx_buf = [FDB_BYTE_ERASED; core::mem::size_of::<LogIdxData>()];
        write_time_ne(&mut idx_buf, LOG_IDX_TS_OFFSET, time);
        #[cfg(not(feature = "fixed_blob_size"))]
        {
            // c: fdb_tsdb.c:359-360
            write_u32_ne(&mut idx_buf, LOG_IDX_TS_OFFSET + TSL_FDBTIME_SIZE, blob.size as u32);
            write_u32_ne(
                &mut idx_buf,
                LOG_IDX_TS_OFFSET + TSL_FDBTIME_SIZE + 4,
                log_addr,
            );
        }

        // c: fdb_tsdb.c:365 — write status = PRE_WRITE (phase 1)
        let mut status_table = [0u8; TSL_STATUS_TABLE_SIZE];
        write_status(
            flash,
            idx_addr,
            &mut status_table,
            FDB_TSL_STATUS_NUM as usize,
            FdbTslStatus::PreWrite as usize,
        )?;

        // c: fdb_tsdb.c:367 — write other index info (time + log_len + log_addr)
        let write_start = LOG_IDX_TS_OFFSET;
        let write_len = core::mem::size_of::<LogIdxData>() - LOG_IDX_TS_OFFSET;
        flash_write(
            flash,
            idx_addr + write_start as u32,
            &idx_buf[write_start..write_start + write_len],
        )?;

        // c: fdb_tsdb.c:369 — write blob data (aligned)
        flash_write_align(flash, log_addr, &blob.buf[..blob.size])?;

        // c: fdb_tsdb.c:374 — write status = WRITE (phase 2, completes the write)
        write_status(
            flash,
            idx_addr,
            &mut status_table,
            FDB_TSL_STATUS_NUM as usize,
            FdbTslStatus::Write as usize,
        )?;

        Ok(())
    }

    /// c: fdb_tsdb.c:379-449 — update_sec_status
    ///
    /// Check if the current sector has enough space for the new TSL. If not,
    /// close the current sector (save end_info, mark FULL), switch to the
    /// next sector (formatting it if necessary), and mark it USING. If the
    /// sector is already FULL, return `SavedFull`.
    fn update_sec_status<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        blob: &FdbBlob,
        cur_time: FdbTime,
    ) -> Result<(), FdbErr> {
        // c: fdb_tsdb.c:386 — sector full check
        if self.cur_sec.status == FdbSectorStoreStatus::Using
            && (self.cur_sec.remain as u32) < LOG_IDX_DATA_SIZE as u32 + wg_align(blob.size as u32)
        {
            // c: fdb_tsdb.c:388
            let cur_sec_addr = self.cur_sec.addr;
            let end_index_temp = self.cur_sec.empty_idx - LOG_IDX_DATA_SIZE as u32;

            // c: fdb_tsdb.c:390-391 — prepare index buffer
            let mut index_buf = [FDB_BYTE_ERASED; TSL_UINT32_ALIGN_SIZE];
            let index_bytes = end_index_temp.to_ne_bytes();
            index_buf[..4].copy_from_slice(&index_bytes);

            // c: fdb_tsdb.c:393-394 — prepare time buffer
            let mut time_buf = [FDB_BYTE_ERASED; TSL_TIME_ALIGN_SIZE];
            write_time_ne(&mut time_buf, 0, self.last_time);

            // c: fdb_tsdb.c:396-407 — save the end node index and timestamp
            let mut end_status_table = [0u8; TSL_STATUS_TABLE_SIZE];
            if self.cur_sec.end_info_stat[0] == FdbTslStatus::Unused {
                // c: fdb_tsdb.c:398-401
                write_status(
                    flash,
                    cur_sec_addr + SECTOR_END0_STATUS_OFFSET as u32,
                    &mut end_status_table,
                    FDB_TSL_STATUS_NUM as usize,
                    FdbTslStatus::PreWrite as usize,
                )?;
                flash_write(
                    flash,
                    cur_sec_addr + SECTOR_END0_TIME_OFFSET as u32,
                    &time_buf,
                )?;
                flash_write(
                    flash,
                    cur_sec_addr + SECTOR_END0_IDX_OFFSET as u32,
                    &index_buf,
                )?;
                write_status(
                    flash,
                    cur_sec_addr + SECTOR_END0_STATUS_OFFSET as u32,
                    &mut end_status_table,
                    FDB_TSL_STATUS_NUM as usize,
                    FdbTslStatus::Write as usize,
                )?;
            } else if self.cur_sec.end_info_stat[1] == FdbTslStatus::Unused {
                // c: fdb_tsdb.c:402-406
                write_status(
                    flash,
                    cur_sec_addr + SECTOR_END1_STATUS_OFFSET as u32,
                    &mut end_status_table,
                    FDB_TSL_STATUS_NUM as usize,
                    FdbTslStatus::PreWrite as usize,
                )?;
                flash_write(
                    flash,
                    cur_sec_addr + SECTOR_END1_TIME_OFFSET as u32,
                    &time_buf,
                )?;
                flash_write(
                    flash,
                    cur_sec_addr + SECTOR_END1_IDX_OFFSET as u32,
                    &index_buf,
                )?;
                write_status(
                    flash,
                    cur_sec_addr + SECTOR_END1_STATUS_OFFSET as u32,
                    &mut end_status_table,
                    FDB_TSL_STATUS_NUM as usize,
                    FdbTslStatus::Write as usize,
                )?;
            }

            // c: fdb_tsdb.c:408-410 — change current sector to full
            let mut status_table = [0u8; FDB_STORE_STATUS_TABLE_SIZE];
            write_status(
                flash,
                cur_sec_addr,
                &mut status_table,
                FDB_SECTOR_STORE_STATUS_NUM as usize,
                FdbSectorStoreStatus::Full as usize,
            )?;
            self.cur_sec.status = FdbSectorStoreStatus::Full;

            // c: fdb_tsdb.c:411-420 — calculate next sector address
            let new_sec_addr = if self.cur_sec.addr + self.parent.sec_size < self.parent.max_size {
                self.cur_sec.addr + self.parent.sec_size
            } else if self.rollover {
                0
            } else {
                // c: fdb_tsdb.c:418-419 — not rollover
                return Err(FdbErr::SavedFull);
            };

            // c: fdb_tsdb.c:421 — read next sector info (into local to avoid borrow conflict)
            // NOTE: C ignores the return value of read_sector_info here; if the
            // new sector has bad magic, status stays at default (Unused != Empty),
            // which triggers the format path below. We match that behaviour.
            let mut new_sec = TsdbSecInfo::default();
            let _ = self.read_sector_info(flash, new_sec_addr, &mut new_sec, false);

            // c: fdb_tsdb.c:422-431 — if next sector is not empty, format it
            if new_sec.status != FdbSectorStoreStatus::Empty {
                // c: fdb_tsdb.c:424-428 — calculate the oldest sector address
                self.parent.oldest_addr = if new_sec_addr + self.parent.sec_size
                    < self.parent.max_size
                {
                    new_sec_addr + self.parent.sec_size
                } else {
                    0
                };
                // c: fdb_tsdb.c:429 — format_sector (C ignores return value)
                let _ = self.format_sector(flash, new_sec_addr);
                // c: fdb_tsdb.c:430 — re-read the formatted sector (C ignores return)
                let _ = self.read_sector_info(flash, new_sec_addr, &mut new_sec, false);
            }

            // Update cur_sec to the new sector
            self.cur_sec = new_sec;
        } else if self.cur_sec.status == FdbSectorStoreStatus::Full {
            // c: fdb_tsdb.c:432-434 — database full
            return Err(FdbErr::SavedFull);
        }

        // c: fdb_tsdb.c:437-446 — if sector is empty, change to using
        if self.cur_sec.status == FdbSectorStoreStatus::Empty {
            // c: fdb_tsdb.c:439-440
            self.cur_sec.status = FdbSectorStoreStatus::Using;
            self.cur_sec.start_time = cur_time;
            // c: fdb_tsdb.c:441
            let mut status_table = [0u8; FDB_STORE_STATUS_TABLE_SIZE];
            write_status(
                flash,
                self.cur_sec.addr,
                &mut status_table,
                FDB_SECTOR_STORE_STATUS_NUM as usize,
                FdbSectorStoreStatus::Using as usize,
            )?;
            // c: fdb_tsdb.c:443-445 — save the start timestamp
            let mut time_buf = [FDB_BYTE_ERASED; TSL_TIME_ALIGN_SIZE];
            write_time_ne(&mut time_buf, 0, cur_time);
            flash_write(
                flash,
                self.cur_sec.addr + SECTOR_START_TIME_OFFSET as u32,
                &time_buf,
            )?;
        }

        Ok(())
    }

    /// c: fdb_tsdb.c:451-499 — tsl_append (internal)
    ///
    /// Append a new TSL to the database. Validates blob size and timestamp
    /// monotonicity, then updates sector status and writes the TSL.
    fn tsl_append_inner<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        blob: &FdbBlob,
        timestamp: Option<FdbTime>,
    ) -> Result<(), FdbErr> {
        // c: fdb_tsdb.c:454 — cur_time = timestamp == NULL ? db->get_time() : *timestamp
        let cur_time = match timestamp {
            Some(ts) => ts,
            None => (self.get_time)(),
        };

        // c: fdb_tsdb.c:456-468 — check blob size
        #[cfg(feature = "fixed_blob_size")]
        {
            if blob.size != FDB_TSDB_FIXED_BLOB_SIZE {
                return Err(FdbErr::WriteErr);
            }
        }
        #[cfg(not(feature = "fixed_blob_size"))]
        {
            if blob.size > self.max_len {
                return Err(FdbErr::WriteErr);
            }
        }

        // c: fdb_tsdb.c:471-476 — check timestamp monotonicity
        if cur_time <= self.last_time {
            return Err(FdbErr::WriteErr);
        }

        // c: fdb_tsdb.c:478-482
        self.update_sec_status(flash, blob, cur_time)?;

        // c: fdb_tsdb.c:483-488
        self.write_tsl(flash, blob, cur_time)?;

        // c: fdb_tsdb.c:490-496 — recalculate the current using sector info
        self.cur_sec.end_idx = self.cur_sec.empty_idx;
        self.cur_sec.end_time = cur_time;
        self.cur_sec.empty_idx += LOG_IDX_DATA_SIZE as u32;
        self.cur_sec.empty_data -= wg_align(blob.size as u32);
        self.cur_sec.remain -= LOG_IDX_DATA_SIZE + wg_align(blob.size as u32) as usize;
        self.last_time = cur_time;

        Ok(())
    }

    /// c: fdb_tsdb.c:509-523 — fdb_tsl_append
    ///
    /// Append a new log to TSDB using the database's `get_time` function.
    pub fn tsl_append<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        blob: &FdbBlob,
    ) -> Result<(), FdbErr> {
        if !self.parent.init_ok {
            return Err(FdbErr::InitFailed);
        }
        self.db_lock();
        let result = self.tsl_append_inner(flash, blob, None);
        self.db_unlock();
        result
    }

    /// c: fdb_tsdb.c:533-547 — fdb_tsl_append_with_ts
    ///
    /// Append a new log to TSDB with a specific timestamp.
    pub fn tsl_append_with_ts<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        blob: &FdbBlob,
        ts: FdbTime,
    ) -> Result<(), FdbErr> {
        if !self.parent.init_ok {
            return Err(FdbErr::InitFailed);
        }
        self.db_lock();
        let result = self.tsl_append_inner(flash, blob, Some(ts));
        self.db_unlock();
        result
    }

    /// c: fdb_tsdb.c:857-864 — fdb_tsl_to_blob
    ///
    /// Convert a TSL object to a blob object by setting the blob's saved
    /// address/length fields from the TSL. Does not perform flash I/O.
    pub fn tsl_to_blob(&self, tsl: &FdbTsl, blob: &mut FdbBlob) -> usize {
        blob.saved_addr = tsl.addr_log;
        blob.saved_meta_addr = tsl.addr_index;
        blob.saved_len = tsl.log_len as usize;
        blob.saved_len
    }

    // ===== T16: Init / deinit / control / clean =====

    /// c: fdb_tsdb.c:902-915 — tsl_format_all
    ///
    /// Format all sectors in the database. After formatting, `oldest_addr`
    /// and `cur_sec.addr` are reset to 0, and `last_time` is reset to 0.
    fn tsl_format_all<F: FlashDevice>(&mut self, flash: &mut F) {
        // c: fdb_tsdb.c:904-907 — sector_iterator with format_all_cb
        let mut sector = TsdbSecInfo::default();
        sector.addr = 0;
        let mut traversed_len = 0u32;
        loop {
            // c: sector_iterator calls read_sector_info (ignoring errors)
            let _ = self.read_sector_info(flash, sector.addr, &mut sector, false);
            // c: format_all_cb — format each sector
            let _ = self.format_sector(flash, sector.addr);
            traversed_len += self.parent.sec_size;
            let next = self.get_next_sector_addr(&sector, traversed_len);
            if next == FAILED_ADDR {
                break;
            }
            sector.addr = next;
        }
        // c: fdb_tsdb.c:908-910
        self.parent.oldest_addr = 0;
        self.cur_sec.addr = 0;
        self.last_time = 0;
        // c: fdb_tsdb.c:912 — read the current using sector info
        let addr = self.cur_sec.addr;
        let mut sec = TsdbSecInfo::default();
        let _ = self.read_sector_info(flash, addr, &mut sec, false);
        self.cur_sec = sec;
    }

    /// c: fdb_tsdb.c:924-929 — fdb_tsl_clean
    ///
    /// Clean all data in the TSDB. This operation is DANGEROUS and not reversible.
    pub fn tsl_clean<F: FlashDevice>(&mut self, flash: &mut F) {
        self.db_lock();
        self.tsl_format_all(flash);
        self.db_unlock();
    }

    /// c: fdb_tsdb.c:1018-1102 — fdb_tsdb_init
    ///
    /// Initialize the time series database. Formats all sectors on first use
    /// or when sector headers are corrupted. Recovers the current sector and
    /// last timestamp from existing data.
    pub fn init<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        name: &'static str,
        path: &'static str,
        get_time: FdbGetTime,
        max_len: usize,
    ) -> Result<(), FdbErr> {
        // c: fdb_tsdb.c:1024 — FDB_ASSERT(get_time)
        // In Rust, fn pointers cannot be null, so no assert needed.

        // c: fdb_tsdb.c:1026-1029 — _fdb_init_ex
        let result = init_ex(&mut self.parent, name, path, FdbDbType::Ts);
        if result.is_err() {
            init_finish(&mut self.parent, result);
            return result;
        }

        // c: fdb_tsdb.c:1032 — lock the TSDB
        self.db_lock();

        // c: fdb_tsdb.c:1034-1039
        self.get_time = get_time;
        self.max_len = max_len;
        self.rollover = true;
        self.parent.oldest_addr = FDB_DATA_UNUSED;
        self.cur_sec.addr = FDB_DATA_UNUSED;

        // c: fdb_tsdb.c:1041 — must less than sector size
        assert!(
            max_len < self.parent.sec_size as usize,
            "max_len must be less than sec_size"
        );

        // c: fdb_tsdb.c:1044-1045 — check all sector headers
        // Inlined sector_iterator + check_sec_hdr_cb (c: fdb_tsdb.c:866-892)
        let mut check_failed = false;
        let mut empty_num: u32 = 0;
        let mut empty_addr: u32 = 0;

        let mut sector = TsdbSecInfo::default();
        sector.addr = 0;
        let mut traversed_len = 0u32;

        loop {
            // c: sector_iterator with traversal=true
            let _ = self.read_sector_info(flash, sector.addr, &mut sector, true);

            // c: check_sec_hdr_cb logic (FDB_SECTOR_STORE_UNUSED matches all)
            if !sector.check_ok {
                // c: fdb_tsdb.c:871-874
                check_failed = true;
                break;
            } else if sector.status == FdbSectorStoreStatus::Using {
                // c: fdb_tsdb.c:875-882
                if self.cur_sec.addr == FDB_DATA_UNUSED {
                    self.cur_sec = sector.clone();
                } else {
                    check_failed = true;
                    break;
                }
            } else if sector.status == FdbSectorStoreStatus::Empty {
                // c: fdb_tsdb.c:883-889
                empty_num += 1;
                empty_addr = sector.addr;
                if empty_num == 1 && self.cur_sec.addr == FDB_DATA_UNUSED {
                    self.cur_sec = sector.clone();
                }
            }

            traversed_len += self.parent.sec_size;
            let next = self.get_next_sector_addr(&sector, traversed_len);
            if next == FAILED_ADDR {
                break;
            }
            sector.addr = next;
        }

        // c: fdb_tsdb.c:1047-1073
        if check_failed {
            if self.parent.not_formatable {
                // c: fdb_tsdb.c:1048-1050
                self.db_unlock();
                let result = Err(FdbErr::ReadErr);
                init_finish(&mut self.parent, result);
                return result;
            } else {
                // c: fdb_tsdb.c:1051-1053 — tsl_format_all
                self.tsl_format_all(flash);
            }
        } else {
            // c: fdb_tsdb.c:1055-1072 — calculate oldest_addr
            let latest_addr = if empty_num > 0 {
                // c: fdb_tsdb.c:1057
                empty_addr
            } else if self.rollover {
                // c: fdb_tsdb.c:1059-1060
                self.cur_sec.addr
            } else {
                // c: fdb_tsdb.c:1062-1063 — no empty sector, no rollover
                self.cur_sec.addr = self.parent.max_size - self.parent.sec_size;
                self.cur_sec.addr
            };

            // c: fdb_tsdb.c:1067-1072
            self.parent.oldest_addr = if latest_addr + self.parent.sec_size >= self.parent.max_size
            {
                0
            } else {
                latest_addr + self.parent.sec_size
            };
        }

        // c: fdb_tsdb.c:1077 — read the current using sector info (with traversal)
        let cur_sec_addr = self.cur_sec.addr;
        let mut cur_sec = TsdbSecInfo::default();
        let _ = self.read_sector_info(flash, cur_sec_addr, &mut cur_sec, true);
        self.cur_sec = cur_sec;

        // c: fdb_tsdb.c:1079-1092 — get last save time
        if self.cur_sec.status == FdbSectorStoreStatus::Using {
            // c: fdb_tsdb.c:1080
            self.last_time = self.cur_sec.end_time;
        } else if self.cur_sec.status == FdbSectorStoreStatus::Empty
            && self.parent.oldest_addr != self.cur_sec.addr
        {
            // c: fdb_tsdb.c:1081-1092
            let addr = if self.cur_sec.addr == 0 {
                self.parent.max_size - self.parent.sec_size
            } else {
                self.cur_sec.addr - self.parent.sec_size
            };
            let mut sec = TsdbSecInfo::default();
            let _ = self.read_sector_info(flash, addr, &mut sec, false);
            self.last_time = sec.end_time;
        }

        // c: fdb_tsdb.c:1095 — unlock
        self.db_unlock();

        // c: fdb_tsdb.c:1099 — _fdb_init_finish
        let result = Ok(());
        init_finish(&mut self.parent, result);
        result
    }

    /// c: fdb_tsdb.c:1111-1116 — fdb_tsdb_deinit
    ///
    /// Deinitialize the time series database.
    pub fn deinit(&mut self) -> Result<(), FdbErr> {
        db_deinit(&mut self.parent);
        Ok(())
    }

    // ===== Builder pattern: type-safe setters/getters (c: fdb_tsdb.c:938-1003) =====
    //
    // Replaces the C `fdb_tsdb_control(db, cmd, arg)` command-pattern switch
    // with individual type-safe setter/getter methods, per the Plan's "Builder
    // 模式 + type-safe setter" requirement.

    /// c: fdb_tsdb.c:943-947 — FDB_TSDB_CTRL_SET_SEC_SIZE
    ///
    /// Set the sector size. MUST be called before `init`.
    pub fn set_sec_size(&mut self, sec_size: u32) -> &mut Self {
        assert!(!self.parent.init_ok, "sec_size must be set before init");
        self.parent.sec_size = sec_size;
        self
    }

    /// c: fdb_tsdb.c:948-950 — FDB_TSDB_CTRL_GET_SEC_SIZE
    pub fn sec_size(&self) -> u32 {
        self.parent.sec_size
    }

    /// c: fdb_tsdb.c:951-960 — FDB_TSDB_CTRL_SET_LOCK
    ///
    /// Set the lock callback. The callback receives `&mut FdbDb`.
    pub fn set_lock(&mut self, lock: fn(&mut FdbDb)) -> &mut Self {
        self.parent.lock = Some(lock);
        self
    }

    /// c: fdb_tsdb.c:961-970 — FDB_TSDB_CTRL_SET_UNLOCK
    pub fn set_unlock(&mut self, unlock: fn(&mut FdbDb)) -> &mut Self {
        self.parent.unlock = Some(unlock);
        self
    }

    /// c: fdb_tsdb.c:971-975 — FDB_TSDB_CTRL_SET_ROLLOVER
    ///
    /// Set the rollover flag. MUST be called after `init`.
    pub fn set_rollover(&mut self, rollover: bool) -> &mut Self {
        assert!(self.parent.init_ok, "rollover must be set after init");
        self.rollover = rollover;
        self
    }

    /// c: fdb_tsdb.c:976-978 — FDB_TSDB_CTRL_GET_ROLLOVER
    pub fn rollover(&self) -> bool {
        self.rollover
    }

    /// c: fdb_tsdb.c:979-981 — FDB_TSDB_CTRL_GET_LAST_TIME
    pub fn last_time(&self) -> FdbTime {
        self.last_time
    }

    /// c: fdb_tsdb.c:982-990 — FDB_TSDB_CTRL_SET_FILE_MODE
    ///
    /// Set the file mode flag. MUST be called before `init`.
    pub fn set_file_mode(&mut self, file_mode: bool) -> &mut Self {
        assert!(!self.parent.init_ok, "file_mode must be set before init");
        self.parent.file_mode = file_mode;
        self
    }

    /// c: fdb_tsdb.c:991-997 — FDB_TSDB_CTRL_SET_MAX_SIZE
    ///
    /// Set the max database size. MUST be called before `init`.
    pub fn set_max_size(&mut self, max_size: u32) -> &mut Self {
        assert!(!self.parent.init_ok, "max_size must be set before init");
        self.parent.max_size = max_size;
        self
    }

    /// c: fdb_tsdb.c:998-1002 — FDB_TSDB_CTRL_SET_NOT_FORMAT
    ///
    /// Set the not-formatable flag. MUST be called before `init`.
    pub fn set_not_formatable(&mut self, not_formatable: bool) -> &mut Self {
        assert!(!self.parent.init_ok, "not_formatable must be set before init");
        self.parent.not_formatable = not_formatable;
        self
    }

    // ===== T17: Iter / query / set_status =====

    /// c: fdb_tsdb.c:556-597 — fdb_tsl_iter
    ///
    /// Iterate all TSLs in forward order (oldest to newest). The callback
    /// receives each TSL; returning `true` stops the iteration.
    ///
    /// NOTE: The C code calls `db_lock`/`db_unlock` here. In Rust, `&self`
    /// methods cannot invoke the `&mut self` lock callbacks. The caller is
    /// responsible for synchronization in multi-threaded environments.
    pub fn tsl_iter<F: FlashDevice>(
        &self,
        flash: &F,
        mut cb: impl FnMut(&FdbTsl) -> bool,
    ) {
        // c: fdb_tsdb.c:562-564
        if !self.parent.init_ok {
            return;
        }

        let mut sec_addr = self.parent.oldest_addr;
        let mut traversed_len = 0u32;

        loop {
            // c: fdb_tsdb.c:574
            traversed_len += self.parent.sec_size;
            let mut sector = TsdbSecInfo::default();
            // c: fdb_tsdb.c:575-577 — read_sector_info (continue on error)
            if self.read_sector_info(flash, sec_addr, &mut sector, false).is_ok() {
                // c: fdb_tsdb.c:579 — sector has TSL
                if sector.status == FdbSectorStoreStatus::Using
                    || sector.status == FdbSectorStoreStatus::Full
                {
                    // c: fdb_tsdb.c:580-583 — copy cur_sec for USING sector
                    if sector.status == FdbSectorStoreStatus::Using {
                        sector = self.cur_sec.clone();
                    }
                    // c: fdb_tsdb.c:584
                    let mut tsl = FdbTsl::default();
                    tsl.addr_index = sector.addr + SECTOR_HDR_DATA_SIZE as u32;
                    // c: fdb_tsdb.c:586-593 — search all TSL
                    loop {
                        self.read_tsl(flash, &mut tsl);
                        if cb(&tsl) {
                            return; // c: callback returned true → stop
                        }
                        let next = FdbTsdb::get_next_tsl_addr(&sector, &tsl);
                        if next == FAILED_ADDR {
                            break;
                        }
                        tsl.addr_index = next;
                    }
                }
            }
            // c: fdb_tsdb.c:595 — get_next_sector_addr
            let next = self.get_next_sector_addr(&sector, traversed_len);
            if next == FAILED_ADDR {
                break;
            }
            sec_addr = next;
        }
    }

    /// c: fdb_tsdb.c:606-649 — fdb_tsl_iter_reverse
    ///
    /// Iterate all TSLs in reverse order (newest to oldest). The callback
    /// receives each TSL; returning `true` stops the iteration.
    pub fn tsl_iter_reverse<F: FlashDevice>(
        &self,
        flash: &F,
        mut cb: impl FnMut(&FdbTsl) -> bool,
    ) {
        // c: fdb_tsdb.c:612-614
        if !self.parent.init_ok {
            return;
        }

        let mut sec_addr = self.cur_sec.addr;
        let mut traversed_len = 0u32;

        loop {
            // c: fdb_tsdb.c:624
            traversed_len += self.parent.sec_size;
            let mut sector = TsdbSecInfo::default();
            // c: fdb_tsdb.c:625-627 — read_sector_info (continue on error)
            if self.read_sector_info(flash, sec_addr, &mut sector, false).is_ok() {
                // c: fdb_tsdb.c:629 — sector has TSL
                if sector.status == FdbSectorStoreStatus::Using
                    || sector.status == FdbSectorStoreStatus::Full
                {
                    // c: fdb_tsdb.c:630-633
                    if sector.status == FdbSectorStoreStatus::Using {
                        sector = self.cur_sec.clone();
                    }
                    // c: fdb_tsdb.c:634
                    let mut tsl = FdbTsl::default();
                    tsl.addr_index = sector.end_idx;
                    // c: fdb_tsdb.c:636-642 — search all TSL (reverse)
                    loop {
                        self.read_tsl(flash, &mut tsl);
                        if cb(&tsl) {
                            return; // c: goto __exit
                        }
                        let next = FdbTsdb::get_last_tsl_addr(&sector, &tsl);
                        if next == FAILED_ADDR {
                            break;
                        }
                        tsl.addr_index = next;
                    }
                } else if sector.status == FdbSectorStoreStatus::Empty
                    || sector.status == FdbSectorStoreStatus::Unused
                {
                    // c: fdb_tsdb.c:643-644 — goto __exit
                    return;
                }
            }
            // c: fdb_tsdb.c:645 — get_last_sector_addr
            let next = self.get_last_sector_addr(&sector, traversed_len);
            if next == FAILED_ADDR {
                break;
            }
            sec_addr = next;
        }
    }

    /// c: fdb_tsdb.c:654-680 — search_start_tsl_addr
    ///
    /// Binary search for the first TSL address matching the `from` timestamp.
    /// Uses `i32` arithmetic to match the C `int` types. When `from > to`
    /// (reverse iteration), adjusts the start position backward if the found
    /// TSL's time exceeds `from`.
    fn search_start_tsl_addr<F: FlashDevice>(
        &self,
        flash: &F,
        start: u32,
        end: u32,
        from: FdbTime,
        to: FdbTime,
    ) -> u32 {
        // c: fdb_tsdb.c:654-680 — C uses `int` for start/end
        let mut start = start as i32;
        let mut end = end as i32;
        let mut tsl = FdbTsl::default();

        loop {
            // c: fdb_tsdb.c:658 — tsl.addr.index = start + FDB_ALIGN((end - start) / 2, LOG_IDX_DATA_SIZE)
            let half = (end - start) / 2;
            let aligned_half = align_up(half as u32, LOG_IDX_DATA_SIZE as u32) as i32;
            tsl.addr_index = (start + aligned_half) as u32;
            self.read_tsl(flash, &mut tsl);

            if tsl.time < from {
                // c: fdb_tsdb.c:660-661
                start = tsl.addr_index as i32 + LOG_IDX_DATA_SIZE as i32;
            } else if tsl.time > from {
                // c: fdb_tsdb.c:662-663
                end = tsl.addr_index as i32 - LOG_IDX_DATA_SIZE as i32;
            } else {
                // c: fdb_tsdb.c:664-665 — exact match
                return tsl.addr_index;
            }

            // c: fdb_tsdb.c:668-677
            if start > end {
                if from > to {
                    // c: fdb_tsdb.c:670-674 — reverse iteration adjustment
                    tsl.addr_index = start as u32;
                    self.read_tsl(flash, &mut tsl);
                    if tsl.time > from {
                        start -= LOG_IDX_DATA_SIZE as i32;
                    }
                }
                break;
            }
        }

        start as u32
    }

    /// c: fdb_tsdb.c:691-769 — fdb_tsl_iter_by_time
    ///
    /// Iterate TSLs within a time range. When `from <= to`, iterates forward
    /// (oldest to newest); when `from > to`, iterates reverse (newest to
    /// oldest). Uses binary search (`search_start_tsl_addr`) to find the
    /// starting TSL within each sector.
    pub fn tsl_iter_by_time<F: FlashDevice>(
        &self,
        flash: &F,
        from: FdbTime,
        to: FdbTime,
        mut cb: impl FnMut(&FdbTsl) -> bool,
    ) {
        // c: fdb_tsdb.c:701-703
        if !self.parent.init_ok {
            return;
        }

        // c: fdb_tsdb.c:705-713 — select direction
        let forward = from <= to;
        let start_addr = if forward {
            self.parent.oldest_addr
        } else {
            self.cur_sec.addr
        };

        let mut sec_addr = start_addr;
        let mut traversed_len = 0u32;
        let mut found_start_tsl = false;

        loop {
            // c: fdb_tsdb.c:726
            traversed_len += self.parent.sec_size;
            let mut sector = TsdbSecInfo::default();
            // c: fdb_tsdb.c:727-729 — read_sector_info (continue on error)
            if self.read_sector_info(flash, sec_addr, &mut sector, false).is_ok() {
                // c: fdb_tsdb.c:731 — sector has TSL
                if sector.status == FdbSectorStoreStatus::Using
                    || sector.status == FdbSectorStoreStatus::Full
                {
                    // c: fdb_tsdb.c:732-735
                    if sector.status == FdbSectorStoreStatus::Using {
                        sector = self.cur_sec.clone();
                    }
                    // c: fdb_tsdb.c:736-740 — check if this sector overlaps the target range
                    let should_search = found_start_tsl
                        || (!found_start_tsl
                            && (if forward {
                                (sec_addr == start_addr && from <= sector.start_time)
                                    || from <= sector.end_time
                            } else {
                                (sec_addr == start_addr && from >= sector.end_time)
                                    || from >= sector.start_time
                            }));

                    if should_search {
                        // c: fdb_tsdb.c:741
                        let start = sector.addr + SECTOR_HDR_DATA_SIZE as u32;
                        let end = sector.end_idx;

                        found_start_tsl = true;
                        // c: fdb_tsdb.c:745 — search the first start TSL address
                        let mut tsl = FdbTsl::default();
                        tsl.addr_index = self.search_start_tsl_addr(flash, start, end, from, to);

                        // c: fdb_tsdb.c:747-760 — search all TSL
                        loop {
                            self.read_tsl(flash, &mut tsl);
                            if tsl.status != FdbTslStatus::Unused {
                                // c: fdb_tsdb.c:750-751 — check time range
                                let in_range = if forward {
                                    tsl.time >= from && tsl.time <= to
                                } else {
                                    tsl.time <= from && tsl.time >= to
                                };
                                if in_range {
                                    // c: fdb_tsdb.c:753-754
                                    if cb(&tsl) {
                                        return; // c: goto __exit
                                    }
                                } else {
                                    // c: fdb_tsdb.c:756-757 — out of range → stop
                                    return; // c: goto __exit
                                }
                            }
                            // c: fdb_tsdb.c:760 — get_tsl_addr (forward or reverse)
                            let next = if forward {
                                FdbTsdb::get_next_tsl_addr(&sector, &tsl)
                            } else {
                                FdbTsdb::get_last_tsl_addr(&sector, &tsl)
                            };
                            if next == FAILED_ADDR {
                                break;
                            }
                            tsl.addr_index = next;
                        }
                    }
                } else if sector.status == FdbSectorStoreStatus::Empty {
                    // c: fdb_tsdb.c:762-763 — goto __exit
                    return;
                }
            }
            // c: fdb_tsdb.c:765 — get_sector_addr (forward or reverse)
            let next = if forward {
                self.get_next_sector_addr(&sector, traversed_len)
            } else {
                self.get_last_sector_addr(&sector, traversed_len)
            };
            if next == FAILED_ADDR {
                break;
            }
            sec_addr = next;
        }
    }

    /// c: fdb_tsdb.c:790-805 — fdb_tsl_query_count
    ///
    /// Query the count of TSLs matching `status` within the time range
    /// [`from`, `to`].
    pub fn tsl_query_count<F: FlashDevice>(
        &self,
        flash: &F,
        from: FdbTime,
        to: FdbTime,
        status: FdbTslStatus,
    ) -> usize {
        // c: fdb_tsdb.c:796-799
        if !self.parent.init_ok {
            return 0;
        }
        // c: fdb_tsdb.c:792-794 + query_count_cb (c: fdb_tsdb.c:771-780)
        let mut count: usize = 0;
        self.tsl_iter_by_time(flash, from, to, |tsl| {
            if tsl.status == status {
                count += 1;
            }
            false // continue iteration
        });
        count
    }

    /// c: fdb_tsdb.c:814-827 — fdb_tsl_max_blob_count
    ///
    /// Get the maximum number of TSL blobs the database can hold, assuming
    /// all blobs are `max_len` (or `FDB_TSDB_FIXED_BLOB_SIZE` if defined).
    pub fn tsl_max_blob_count(&self) -> usize {
        // c: fdb_tsdb.c:816-820
        #[cfg(feature = "fixed_blob_size")]
        let max_blob_len = FDB_TSDB_FIXED_BLOB_SIZE as u32;
        #[cfg(not(feature = "fixed_blob_size"))]
        let max_blob_len = self.max_len as u32;

        // c: fdb_tsdb.c:822-826
        let sec_size = self.parent.sec_size as usize - SECTOR_HDR_DATA_SIZE;
        let blob_size = LOG_IDX_DATA_SIZE + wg_align(max_blob_len) as usize;
        let n_sec = (self.parent.max_size / self.parent.sec_size) as usize;

        n_sec * (sec_size / blob_size)
    }

    /// c: fdb_tsdb.c:838-847 — fdb_tsl_set_status
    ///
    /// Set the status of a TSL node. Writes the status to flash at the TSL's
    /// index address.
    pub fn tsl_set_status<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        tsl: &FdbTsl,
        status: FdbTslStatus,
    ) -> Result<(), FdbErr> {
        // c: fdb_tsdb.c:841
        let mut status_table = [0u8; TSL_STATUS_TABLE_SIZE];
        // c: fdb_tsdb.c:844 — _FDB_WRITE_STATUS
        write_status(
            flash,
            tsl.addr_index,
            &mut status_table,
            FDB_TSL_STATUS_NUM as usize,
            status as usize,
        )?;
        Ok(())
    }
}

// ==========================================================================
// Unit tests
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::def::{
        FdbSectorStoreStatus, FdbTsl, FdbTslStatus, TsdbSecInfo, FDB_SECTOR_STORE_STATUS_NUM,
        FDB_STORE_STATUS_TABLE_SIZE, FDB_TSL_STATUS_NUM,
    };
    use crate::low_lvl::{blob_make, blob_read, status_table_size, wg_align};
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

    // ---- T15: Core read/write tests ----

    /// Helper: create a FdbTsdb backed by MockFlash with sector 0 formatted
    /// and ready for appending.
    fn setup_tsdb(sec_size: u32, max_size: u32, max_len: usize) -> (FdbTsdb, MockFlash) {
        let mut flash = MockFlash::new("test", sec_size, max_size, sec_size);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = sec_size;
        db.parent.max_size = max_size;
        db.parent.init_ok = true;
        db.max_len = max_len;
        db.get_time = || 1000;
        db.rollover = true;
        db.parent.oldest_addr = 0;

        // Format sector 0 and read its info
        db.format_sector(&mut flash, 0).unwrap();
        let mut sec = TsdbSecInfo::default();
        db.read_sector_info(&flash, 0, &mut sec, false).unwrap();
        db.cur_sec = sec;
        db.last_time = 0;

        (db, flash)
    }

    #[test]
    fn test_format_sector() {
        // c: fdb_tsdb.c:309-327 — format_sector writes magic + status=EMPTY
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let db = FdbTsdb::default();
        let db_ref = &db; // read-only for format_sector
        let mut db_mut = FdbTsdb::default();
        db_mut.parent.sec_size = 4096;

        // Format sector 0
        db_mut.format_sector(&mut flash, 0).unwrap();

        // Verify magic word at SECTOR_MAGIC_OFFSET
        let mut magic_buf = [0u8; 4];
        flash
            .read(SECTOR_MAGIC_OFFSET as u32, &mut magic_buf)
            .unwrap();
        assert_eq!(
            u32::from_ne_bytes(magic_buf),
            SECTOR_MAGIC_WORD,
            "magic word must be written at SECTOR_MAGIC_OFFSET"
        );

        // Verify status = EMPTY (index 1) at sector start
        let mut status_buf = [0u8; FDB_STORE_STATUS_TABLE_SIZE];
        flash.read(0, &mut status_buf).unwrap();
        let status_idx = get_status(&status_buf, FDB_SECTOR_STORE_STATUS_NUM as usize);
        assert_eq!(
            sector_store_status_from_index(status_idx),
            FdbSectorStoreStatus::Empty,
            "formatted sector must have EMPTY status"
        );

        // Verify sector is erased except for header
        let mut buf = [0u8; 16];
        flash.read(100, &mut buf).unwrap();
        assert_eq!(buf, [0xFFu8; 16], "rest of sector must be erased");

        // format_sector with non-aligned addr should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut f = MockFlash::new("t", 4096, 16384, 4096);
            let mut d = FdbTsdb::default();
            d.parent.sec_size = 4096;
            d.format_sector(&mut f, 100)
        }));
        assert!(result.is_err(), "non-aligned addr must panic");
        let _ = db_ref; // suppress unused warning
    }

    #[test]
    fn test_read_sector_info() {
        // c: fdb_tsdb.c:242-307 — read_sector_info on a formatted sector
        let (db, flash) = setup_tsdb(4096, 16384, 256);

        let mut sec = TsdbSecInfo::default();
        db.read_sector_info(&flash, 0, &mut sec, false).unwrap();

        assert_eq!(sec.addr, 0);
        assert_eq!(sec.magic, SECTOR_MAGIC_WORD);
        assert!(sec.check_ok, "check_ok must be true for valid magic");
        assert_eq!(sec.status, FdbSectorStoreStatus::Empty);
        assert_eq!(
            sec.empty_idx,
            SECTOR_HDR_DATA_SIZE as u32,
            "empty_idx must be after sector header"
        );
        assert_eq!(sec.empty_data, 4096, "empty_data must be at sector end");
        assert_eq!(
            sec.remain,
            4096 - SECTOR_HDR_DATA_SIZE,
            "remain must be sector_size - header"
        );
    }

    #[test]
    fn test_read_sector_info_bad_magic() {
        // c: fdb_tsdb.c:256-259 — bad magic returns InitFailed
        let flash = MockFlash::new("test", 4096, 16384, 4096);
        let db = FdbTsdb::default();
        let db_ref = &db;

        // No formatting — flash is all 0xFF, magic won't match
        let mut sec = TsdbSecInfo::default();
        let result = db_ref.read_sector_info(&flash, 0, &mut sec, false);
        assert_eq!(result, Err(FdbErr::InitFailed), "bad magic must return InitFailed");
        assert!(!sec.check_ok, "check_ok must be false for bad magic");
    }

    #[test]
    fn test_append_tsl() {
        // c: fdb_tsdb.c:451-499 — tsl_append + read_tsl roundtrip
        let (mut db, mut flash) = setup_tsdb(4096, 16384, 256);

        // Append a 64-byte blob with timestamp=100
        let mut data_buf = [0xAAu8; 64];
        let mut blob = blob_make(&mut data_buf);
        blob.size = 64;
        let result = db.tsl_append_with_ts(&mut flash, &blob, 100);
        assert!(result.is_ok(), "append should succeed: {:?}", result);

        // Read back the TSL at the index address (sector start + header)
        let mut tsl = FdbTsl::default();
        tsl.addr_index = SECTOR_HDR_DATA_SIZE as u32;
        db.read_tsl(&flash, &mut tsl);

        assert_eq!(tsl.status, FdbTslStatus::Write, "TSL status must be WRITE");
        assert_eq!(tsl.time, 100, "TSL time must match");
        assert_eq!(tsl.log_len, 64, "TSL log_len must match blob size");

        // Read the blob data back via tsl_to_blob + blob_read
        let mut read_buf = [0u8; 64];
        let mut read_blob = blob_make(&mut read_buf);
        db.tsl_to_blob(&tsl, &mut read_blob);
        let read_len = blob_read(&flash, &mut read_blob);
        assert_eq!(read_len, 64, "blob_read must return full length");
        assert_eq!(&read_buf[..64], &data_buf[..64], "blob data must match");
    }

    #[test]
    fn test_timestamp_ordering() {
        // c: fdb_tsdb.c:471-476 — timestamp must be strictly increasing
        let (mut db, mut flash) = setup_tsdb(4096, 16384, 256);

        let mut data = [0u8; 32];
        let mut blob = blob_make(&mut data);
        blob.size = 32;

        // First append with ts=100 succeeds (last_time starts at 0)
        assert!(
            db.tsl_append_with_ts(&mut flash, &blob, 100).is_ok(),
            "first append with ts=100 must succeed"
        );

        // Append with ts=99 (less than last_time=100) must fail
        assert_eq!(
            db.tsl_append_with_ts(&mut flash, &blob, 99),
            Err(FdbErr::WriteErr),
            "ts < last_time must fail"
        );

        // Append with ts=100 (equal to last_time) must fail
        assert_eq!(
            db.tsl_append_with_ts(&mut flash, &blob, 100),
            Err(FdbErr::WriteErr),
            "ts == last_time must fail"
        );

        // Append with ts=200 succeeds
        assert!(
            db.tsl_append_with_ts(&mut flash, &blob, 200).is_ok(),
            "ts > last_time must succeed"
        );
        assert_eq!(db.last_time, 200, "last_time must be updated");
    }

    #[test]
    fn test_append_blob_too_large() {
        // c: fdb_tsdb.c:462-468 — blob size > max_len must fail
        let (mut db, mut flash) = setup_tsdb(4096, 16384, 64);

        let mut data = [0u8; 128];
        let mut blob = blob_make(&mut data);
        blob.size = 128; // > max_len=64
        assert_eq!(
            db.tsl_append_with_ts(&mut flash, &blob, 100),
            Err(FdbErr::WriteErr),
            "blob > max_len must fail"
        );
    }

    #[test]
    fn test_append_not_initialized() {
        // c: fdb_tsdb.c:513-516 — append before init must fail
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut db = FdbTsdb::default();
        db.parent.init_ok = false;

        let mut data = [0u8; 16];
        let mut blob = blob_make(&mut data);
        blob.size = 16;
        assert_eq!(
            db.tsl_append(&mut flash, &blob),
            Err(FdbErr::InitFailed),
            "append before init must fail"
        );
    }

    #[test]
    fn test_get_next_sector_addr() {
        // c: fdb_tsdb.c:177-190
        let (db, _flash) = setup_tsdb(4096, 16384, 256);

        let mut sec = TsdbSecInfo::default();
        sec.addr = 0;

        // Next sector after 0 is 4096
        assert_eq!(db.get_next_sector_addr(&sec, 0), 4096);

        // Next sector after 12288 (sector 3) is 16384-4096=12288 → wraps to 0
        sec.addr = 12288;
        assert_eq!(db.get_next_sector_addr(&sec, 12288), 0, "last sector wraps to 0");

        // Traversed all sectors → FAILED_ADDR
        sec.addr = 12288;
        assert_eq!(
            db.get_next_sector_addr(&sec, 16384),
            FAILED_ADDR,
            "after traversing all sectors, return FAILED_ADDR"
        );
    }

    #[test]
    fn test_get_last_sector_addr() {
        // c: fdb_tsdb.c:227-240
        let (db, _flash) = setup_tsdb(4096, 16384, 256);

        let mut sec = TsdbSecInfo::default();
        sec.addr = 4096;
        assert_eq!(db.get_last_sector_addr(&sec, 0), 0, "previous of sector 1 is sector 0");

        sec.addr = 0;
        assert_eq!(
            db.get_last_sector_addr(&sec, 0),
            12288,
            "previous of sector 0 is the last sector (12288)"
        );

        // Traversed all → FAILED_ADDR
        sec.addr = 0;
        assert_eq!(db.get_last_sector_addr(&sec, 16384), FAILED_ADDR);
    }

    #[test]
    fn test_get_next_and_last_tsl_addr() {
        // c: fdb_tsdb.c:192-225
        let mut sector = TsdbSecInfo::default();
        sector.status = FdbSectorStoreStatus::Using;
        sector.addr = 0;
        sector.end_idx = SECTOR_HDR_DATA_SIZE as u32 + LOG_IDX_DATA_SIZE as u32 * 3; // 3 TSLs

        let mut tsl = FdbTsl::default();
        tsl.addr_index = SECTOR_HDR_DATA_SIZE as u32; // first TSL

        // Next TSL after first
        let next = FdbTsdb::get_next_tsl_addr(&sector, &tsl);
        assert_eq!(
            next,
            SECTOR_HDR_DATA_SIZE as u32 + LOG_IDX_DATA_SIZE as u32,
            "next TSL addr"
        );

        // Last TSL (from 3rd back to 2nd)
        tsl.addr_index = SECTOR_HDR_DATA_SIZE as u32 + LOG_IDX_DATA_SIZE as u32 * 2;
        let last = FdbTsdb::get_last_tsl_addr(&sector, &tsl);
        assert_eq!(
            last,
            SECTOR_HDR_DATA_SIZE as u32 + LOG_IDX_DATA_SIZE as u32,
            "last TSL addr"
        );

        // Empty sector returns FAILED_ADDR
        sector.status = FdbSectorStoreStatus::Empty;
        assert_eq!(FdbTsdb::get_next_tsl_addr(&sector, &tsl), FAILED_ADDR);
        assert_eq!(FdbTsdb::get_last_tsl_addr(&sector, &tsl), FAILED_ADDR);
    }

    #[test]
    fn test_append_multiple_tsls() {
        // Append 5 TSLs and verify they can all be read back in order
        let (mut db, mut flash) = setup_tsdb(4096, 16384, 256);

        for i in 1..=5 {
            let mut data = [i as u8; 32];
            let mut blob = blob_make(&mut data);
            blob.size = 32;
            let ts = i as FdbTime * 100;
            db.tsl_append_with_ts(&mut flash, &blob, ts).unwrap();
        }

        // Verify by reading each TSL
        let mut tsl = FdbTsl::default();
        tsl.addr_index = SECTOR_HDR_DATA_SIZE as u32;
        for i in 1..=5 {
            db.read_tsl(&flash, &mut tsl);
            assert_eq!(tsl.status, FdbTslStatus::Write, "TSL {} status", i);
            assert_eq!(tsl.time, i as FdbTime * 100, "TSL {} time", i);
            assert_eq!(tsl.log_len, 32, "TSL {} log_len", i);
            tsl.addr_index += LOG_IDX_DATA_SIZE as u32;
        }

        // Verify cur_sec tracking
        assert_eq!(db.last_time, 500, "last_time must be 500 after 5 appends");
        assert_eq!(
            db.cur_sec.end_idx,
            SECTOR_HDR_DATA_SIZE as u32 + LOG_IDX_DATA_SIZE as u32 * 4,
            "end_idx must point to last written TSL"
        );
    }

    // ---- T16: Init / deinit / control / clean tests ----

    /// Simple get_time function for testing.
    fn test_get_time() -> FdbTime {
        1000
    }

    #[test]
    fn test_tsdb_init() {
        // c: fdb_tsdb.c:1018-1102 — init on fresh (all-erased) flash
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;

        let result = db.init(&mut flash, "test_tsdb", "test_part", test_get_time, 256);
        assert!(result.is_ok(), "init should succeed on fresh flash: {:?}", result);

        // Verify state after init
        assert!(db.parent.init_ok, "init_ok must be true");
        assert!(db.rollover, "rollover must default to true");
        assert_eq!(db.last_time, 0, "last_time must be 0 on fresh flash");
        assert_eq!(db.parent.oldest_addr, 0, "oldest_addr must be 0");
        assert_eq!(db.cur_sec.addr, 0, "cur_sec.addr must be 0");
        assert_eq!(
            db.cur_sec.status,
            FdbSectorStoreStatus::Empty,
            "cur_sec must be Empty after format_all"
        );
        assert_eq!(db.max_len, 256);
    }

    #[test]
    fn test_tsdb_init_and_append() {
        // Init + append + verify data integrity
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();

        // Append a TSL
        let mut data = [0x55u8; 64];
        let mut blob = blob_make(&mut data);
        blob.size = 64;
        db.tsl_append_with_ts(&mut flash, &blob, 100).unwrap();

        // Verify last_time updated
        assert_eq!(db.last_time, 100);

        // Read back
        let mut tsl = FdbTsl::default();
        tsl.addr_index = SECTOR_HDR_DATA_SIZE as u32;
        db.read_tsl(&flash, &mut tsl);
        assert_eq!(tsl.time, 100);
        assert_eq!(tsl.log_len, 64);
        assert_eq!(tsl.status, FdbTslStatus::Write);
    }

    #[test]
    fn test_tsdb_rollover() {
        // c: fdb_tsdb.c:411-420 — verify circular rollover with 2 sectors
        // sec_size=512, max_size=1024 (2 sectors), blob=32 bytes
        // Each TSL: LOG_IDX_DATA_SIZE(16) + wg_align(32)(32) = 48 bytes
        // Available per sector: 512 - SECTOR_HDR_DATA_SIZE(32) = 480
        // TSLs per sector: 480 / 48 = 10
        let mut flash = MockFlash::new("test", 512, 1024, 512);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 512;
        db.parent.max_size = 1024;
        db.init(&mut flash, "test", "part", test_get_time, 64).unwrap();
        assert!(db.rollover, "rollover must be true by default");

        let tsls_per_sector = (512 - SECTOR_HDR_DATA_SIZE) / (LOG_IDX_DATA_SIZE + wg_align(32) as usize);
        assert_eq!(tsls_per_sector, 10, "expected 10 TSLs per sector");

        // Fill sector 0 (10 TSLs) + sector 1 (10 TSLs) + rollover to sector 0
        for i in 1..=(tsls_per_sector * 2 + 2) {
            let mut data = [i as u8; 32];
            let mut blob = blob_make(&mut data);
            blob.size = 32;
            let ts = i as FdbTime * 10;
            let result = db.tsl_append_with_ts(&mut flash, &blob, ts);
            assert!(
                result.is_ok(),
                "TSL {} should succeed with rollover: {:?}",
                i,
                result
            );
        }

        // Verify last_time is updated
        assert_eq!(
            db.last_time,
            (tsls_per_sector * 2 + 2) as FdbTime * 10,
            "last_time must reflect last append"
        );
    }

    #[test]
    fn test_tsdb_save_full() {
        // c: fdb_tsdb.c:418-419 — non-rollover mode returns SavedFull
        let mut flash = MockFlash::new("test", 512, 1024, 512);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 512;
        db.parent.max_size = 1024;
        db.init(&mut flash, "test", "part", test_get_time, 64).unwrap();
        // Disable rollover (must be after init)
        db.set_rollover(false);
        assert!(!db.rollover);

        let tsls_per_sector = (512 - SECTOR_HDR_DATA_SIZE) / (LOG_IDX_DATA_SIZE + wg_align(32) as usize);

        // Fill both sectors
        for i in 1..=(tsls_per_sector * 2) {
            let mut data = [i as u8; 32];
            let mut blob = blob_make(&mut data);
            blob.size = 32;
            let ts = i as FdbTime * 10;
            db.tsl_append_with_ts(&mut flash, &blob, ts).unwrap();
        }

        // Next append should fail with SavedFull
        let mut data = [0xFFu8; 32];
        let mut blob = blob_make(&mut data);
        blob.size = 32;
        let result = db.tsl_append_with_ts(&mut flash, &blob, 9999);
        assert_eq!(
            result,
            Err(FdbErr::SavedFull),
            "non-rollover full must return SavedFull"
        );
    }

    #[test]
    fn test_tsdb_clean() {
        // c: fdb_tsdb.c:924-929 — clean all data
        let (mut db, mut flash) = setup_tsdb(4096, 16384, 256);

        // Append some data
        for i in 1..=5 {
            let mut data = [i as u8; 32];
            let mut blob = blob_make(&mut data);
            blob.size = 32;
            db.tsl_append_with_ts(&mut flash, &blob, i as FdbTime * 100).unwrap();
        }
        assert_eq!(db.last_time, 500);

        // Clean
        db.tsl_clean(&mut flash);

        // Verify state after clean
        assert_eq!(db.last_time, 0, "last_time must be 0 after clean");
        assert_eq!(db.parent.oldest_addr, 0, "oldest_addr must be 0 after clean");
        assert_eq!(db.cur_sec.addr, 0, "cur_sec.addr must be 0 after clean");
        assert_eq!(
            db.cur_sec.status,
            FdbSectorStoreStatus::Empty,
            "cur_sec must be Empty after clean"
        );

        // Verify can append after clean
        let mut data = [0xAAu8; 16];
        let mut blob = blob_make(&mut data);
        blob.size = 16;
        assert!(
            db.tsl_append_with_ts(&mut flash, &blob, 100).is_ok(),
            "append should succeed after clean"
        );
    }

    #[test]
    fn test_tsdb_deinit() {
        // c: fdb_tsdb.c:1111-1116 — deinit clears init_ok
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();
        assert!(db.parent.init_ok);

        db.deinit().unwrap();
        assert!(!db.parent.init_ok, "deinit must clear init_ok");

        // Append after deinit must fail
        let mut data = [0u8; 16];
        let mut blob = blob_make(&mut data);
        blob.size = 16;
        assert_eq!(
            db.tsl_append(&mut flash, &blob),
            Err(FdbErr::InitFailed),
            "append after deinit must fail"
        );
    }

    #[test]
    fn test_builder_setters() {
        // c: fdb_tsdb.c:938-1003 — Builder pattern setters/getters
        let mut db = FdbTsdb::default();

        // set_sec_size before init
        db.set_sec_size(4096);
        assert_eq!(db.sec_size(), 4096);

        // set_max_size before init
        db.set_max_size(16384);
        assert_eq!(db.parent.max_size, 16384);

        // set_file_mode before init
        db.set_file_mode(true);
        assert!(db.parent.file_mode);

        // set_not_formatable before init
        db.set_not_formatable(true);
        assert!(db.parent.not_formatable);

        // set_lock / set_unlock
        fn test_lock(_db: &mut FdbDb) {}
        fn test_unlock(_db: &mut FdbDb) {}
        db.set_lock(test_lock);
        db.set_unlock(test_unlock);
        assert!(db.parent.lock.is_some());
        assert!(db.parent.unlock.is_some());

        // Init
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        // not_formatable=true with bad flash → init fails with ReadErr
        let result = db.init(&mut flash, "test", "part", test_get_time, 256);
        assert_eq!(
            result,
            Err(FdbErr::ReadErr),
            "not_formatable with corrupted sectors must fail"
        );
    }

    #[test]
    #[should_panic(expected = "sec_size must be set before init")]
    fn test_set_sec_size_after_init_panics() {
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();
        db.set_sec_size(2048); // should panic
    }

    #[test]
    #[should_panic(expected = "rollover must be set after init")]
    fn test_set_rollover_before_init_panics() {
        let mut db = FdbTsdb::default();
        db.set_rollover(false); // should panic
    }

    #[test]
    fn test_init_not_formatable_with_good_flash() {
        // not_formatable=true but flash is pre-formatted → init succeeds
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);

        // Pre-format all sectors
        {
            let mut db = FdbTsdb::default();
            db.parent.sec_size = 4096;
            db.parent.max_size = 16384;
            db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();
            db.deinit().unwrap();
        }

        // Now init with not_formatable=true on pre-formatted flash
        let mut db = FdbTsdb::default();
        db.set_sec_size(4096);
        db.set_max_size(16384);
        db.set_not_formatable(true);
        let result = db.init(&mut flash, "test", "part", test_get_time, 256);
        assert!(result.is_ok(), "init on pre-formatted flash with not_formatable should succeed: {:?}", result);
    }

    // ---- T17: Iter / query / set_status tests ----

    /// Helper: init a TSDB and append TSLs with timestamps [100, 200, ..., 500]
    fn setup_tsdb_with_data() -> (FdbTsdb, MockFlash) {
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();

        for i in 1..=5 {
            let mut data = [i as u8; 32];
            let mut blob = blob_make(&mut data);
            blob.size = 32;
            let ts = i as FdbTime * 100;
            db.tsl_append_with_ts(&mut flash, &blob, ts).unwrap();
        }
        (db, flash)
    }

    #[test]
    fn test_tsl_iter() {
        // c: fdb_tsdb.c:556-597 — forward iteration
        let (db, flash) = setup_tsdb_with_data();

        let mut timestamps = Vec::new();
        db.tsl_iter(&flash, |tsl| {
            timestamps.push(tsl.time);
            false // continue
        });

        assert_eq!(timestamps, vec![100, 200, 300, 400, 500], "forward iter must be in ascending order");
    }

    #[test]
    fn test_tsl_iter_callback_stops() {
        // Callback returning true should stop iteration
        let (db, flash) = setup_tsdb_with_data();

        let mut count = 0;
        db.tsl_iter(&flash, |_tsl| {
            count += 1;
            count >= 3 // stop after 3
        });

        assert_eq!(count, 3, "iteration must stop when callback returns true");
    }

    #[test]
    fn test_tsl_iter_reverse() {
        // c: fdb_tsdb.c:606-649 — reverse iteration
        let (db, flash) = setup_tsdb_with_data();

        let mut timestamps = Vec::new();
        db.tsl_iter_reverse(&flash, |tsl| {
            timestamps.push(tsl.time);
            false
        });

        assert_eq!(timestamps, vec![500, 400, 300, 200, 100], "reverse iter must be in descending order");
    }

    #[test]
    fn test_tsl_iter_reverse_stops() {
        let (db, flash) = setup_tsdb_with_data();

        let mut count = 0;
        db.tsl_iter_reverse(&flash, |_tsl| {
            count += 1;
            count >= 2
        });

        assert_eq!(count, 2, "reverse iteration must stop when callback returns true");
    }

    #[test]
    fn test_tsl_iter_by_time_forward() {
        // c: fdb_tsdb.c:691-769 — range query [200, 400]
        let (db, flash) = setup_tsdb_with_data();

        let mut timestamps = Vec::new();
        db.tsl_iter_by_time(&flash, 200, 400, |tsl| {
            timestamps.push(tsl.time);
            false
        });

        assert_eq!(timestamps, vec![200, 300, 400], "range [200,400] must return 3 TSLs");
    }

    #[test]
    fn test_tsl_iter_by_time_full_range() {
        // Full range [100, 500]
        let (db, flash) = setup_tsdb_with_data();

        let mut count = 0;
        db.tsl_iter_by_time(&flash, 100, 500, |_tsl| {
            count += 1;
            false
        });
        assert_eq!(count, 5, "full range must return all 5 TSLs");
    }

    #[test]
    fn test_tsl_iter_by_time_reverse() {
        // from > to → reverse iteration
        let (db, flash) = setup_tsdb_with_data();

        let mut timestamps = Vec::new();
        db.tsl_iter_by_time(&flash, 400, 200, |tsl| {
            timestamps.push(tsl.time);
            false
        });

        assert_eq!(timestamps, vec![400, 300, 200], "reverse range [400,200] must return 3 TSLs in descending order");
    }

    #[test]
    fn test_tsl_iter_by_time_single() {
        // Range containing a single TSL
        let (db, flash) = setup_tsdb_with_data();

        let mut timestamps = Vec::new();
        db.tsl_iter_by_time(&flash, 300, 300, |tsl| {
            timestamps.push(tsl.time);
            false
        });

        assert_eq!(timestamps, vec![300], "range [300,300] must return exactly 1 TSL");
    }

    #[test]
    fn test_query_count() {
        // c: fdb_tsdb.c:790-805 — count TSLs with WRITE status
        let (db, flash) = setup_tsdb_with_data();

        let count = db.tsl_query_count(&flash, 100, 500, FdbTslStatus::Write);
        assert_eq!(count, 5, "all 5 TSLs have WRITE status");

        let count = db.tsl_query_count(&flash, 200, 400, FdbTslStatus::Write);
        assert_eq!(count, 3, "range [200,400] has 3 TSLs with WRITE status");

        let count = db.tsl_query_count(&flash, 100, 500, FdbTslStatus::Deleted);
        assert_eq!(count, 0, "no TSLs with DELETED status");
    }

    #[test]
    fn test_set_status() {
        // c: fdb_tsdb.c:838-847 — set TSL status and verify persistence
        let (mut db, mut flash) = setup_tsdb_with_data();

        // Find the TSL at timestamp 300 and set it to DELETED
        let mut target_tsl: Option<FdbTsl> = None;
        db.tsl_iter(&flash, |tsl| {
            if tsl.time == 300 {
                target_tsl = Some(*tsl);
                true // stop
            } else {
                false
            }
        });
        let tsl = target_tsl.expect("TSL with time=300 must exist");

        // Set status to DELETED
        db.tsl_set_status(&mut flash, &tsl, FdbTslStatus::Deleted).unwrap();

        // Verify by re-reading the TSL
        let mut reread_tsl = FdbTsl::default();
        reread_tsl.addr_index = tsl.addr_index;
        db.read_tsl(&flash, &mut reread_tsl);
        assert_eq!(
            reread_tsl.status,
            FdbTslStatus::Deleted,
            "TSL status must be DELETED after set_status"
        );

        // Verify query_count reflects the change
        let count_deleted = db.tsl_query_count(&flash, 100, 500, FdbTslStatus::Deleted);
        assert_eq!(count_deleted, 1, "1 TSL should have DELETED status");

        let count_write = db.tsl_query_count(&flash, 100, 500, FdbTslStatus::Write);
        assert_eq!(count_write, 4, "4 TSLs should still have WRITE status");
    }

    #[test]
    fn test_max_blob_count() {
        // c: fdb_tsdb.c:814-827 — max blob count calculation
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();

        // max_blob_count = n_sec * (sec_size - SECTOR_HDR_DATA_SIZE) / (LOG_IDX_DATA_SIZE + wg_align(max_len))
        // = 4 * (4096 - 32) / (16 + 256)
        // = 4 * 4064 / 272
        // = 4 * 14 (integer division)
        // = 56
        let expected = 4 * ((4096 - SECTOR_HDR_DATA_SIZE) / (LOG_IDX_DATA_SIZE + wg_align(256) as usize));
        assert_eq!(db.tsl_max_blob_count(), expected, "max_blob_count must match formula");
        assert!(db.tsl_max_blob_count() > 0, "max_blob_count must be positive");
    }

    #[test]
    fn test_tsl_to_blob() {
        // c: fdb_tsdb.c:857-864 — tsl_to_blob sets saved fields
        let (db, flash) = setup_tsdb_with_data();

        // Get the first TSL
        let mut first_tsl: Option<FdbTsl> = None;
        db.tsl_iter(&flash, |tsl| {
            first_tsl = Some(*tsl);
            true // stop after first
        });
        let tsl = first_tsl.expect("at least one TSL must exist");

        // Convert to blob
        let mut read_buf = [0u8; 64];
        let mut blob = blob_make(&mut read_buf);
        let ret = db.tsl_to_blob(&tsl, &mut blob);

        assert_eq!(ret, tsl.log_len as usize, "return value must be log_len");
        assert_eq!(blob.saved_addr, tsl.addr_log, "saved_addr must match tsl.addr_log");
        assert_eq!(blob.saved_meta_addr, tsl.addr_index, "saved_meta_addr must match tsl.addr_index");
        assert_eq!(blob.saved_len, tsl.log_len as usize, "saved_len must match tsl.log_len");

        // Read the blob data
        let read_len = blob_read(&flash, &mut blob);
        assert_eq!(read_len, 32, "blob_read should return 32 bytes");
        assert_eq!(&read_buf[..32], &[1u8; 32], "blob data should be [1; 32]");
    }

    // ---- T18: Comprehensive edge case tests ----

    #[test]
    fn test_multi_sector_rollover() {
        // c: fdb_tsdb.c:411-420 — 4 sectors with rollover, verify data survives wrap-around
        let mut flash = MockFlash::new("test", 512, 2048, 512); // 4 sectors
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 512;
        db.parent.max_size = 2048;
        db.init(&mut flash, "test", "part", test_get_time, 64).unwrap();

        let tsls_per_sec = (512 - SECTOR_HDR_DATA_SIZE) / (LOG_IDX_DATA_SIZE + wg_align(16) as usize);
        // Append enough TSLs to fill all 4 sectors and wrap around
        let total = tsls_per_sec * 4 + tsls_per_sec; // 5 sector-fulls
        for i in 1..=total {
            let mut data = [(i % 100) as u8; 16];
            let mut blob = blob_make(&mut data);
            blob.size = 16;
            db.tsl_append_with_ts(&mut flash, &blob, i as FdbTime * 10).unwrap();
        }

        // Verify data can be iterated (at least the most recent sector-full of data)
        let mut count = 0;
        db.tsl_iter(&flash, |_tsl| {
            count += 1;
            false
        });
        // After rollover, old data is overwritten. The exact count depends on
        // how many sectors were overwritten. At least 1 sector of data should exist.
        assert!(count > 0, "iter must return TSLs after multi-sector rollover");
    }

    #[test]
    fn test_recovery_pre_write() {
        // c: fdb_tsdb.c:280-304 — PRE_WRITE TSL recovery on reboot
        // Simulate a crash during append: TSL 0 is WRITE, TSL 1 is PRE_WRITE
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();

        // Append one TSL (ts=100, status=WRITE)
        let mut data = [0x55u8; 32];
        let mut blob = blob_make(&mut data);
        blob.size = 32;
        db.tsl_append_with_ts(&mut flash, &blob, 100).unwrap();

        // Simulate crash: write PRE_WRITE status at the next TSL index position
        let next_idx_addr = SECTOR_HDR_DATA_SIZE as u32 + LOG_IDX_DATA_SIZE as u32;
        let mut status_table = [0u8; TSL_STATUS_TABLE_SIZE];
        write_status(
            &mut flash,
            next_idx_addr,
            &mut status_table,
            FDB_TSL_STATUS_NUM as usize,
            FdbTslStatus::PreWrite as usize,
        ).unwrap();

        // Reboot: deinit + init
        db.deinit().unwrap();
        let mut db2 = FdbTsdb::default();
        db2.parent.sec_size = 4096;
        db2.parent.max_size = 16384;
        db2.init(&mut flash, "test", "part", test_get_time, 256).unwrap();

        // Verify last_time is recovered from the WRITE TSL (not PRE_WRITE)
        assert_eq!(
            db2.last_time, 100,
            "last_time must be 100 (from WRITE TSL, not PRE_WRITE which has time=0)"
        );

        // Verify the WRITE TSL is still readable
        let mut tsl = FdbTsl::default();
        tsl.addr_index = SECTOR_HDR_DATA_SIZE as u32;
        db2.read_tsl(&flash, &mut tsl);
        assert_eq!(tsl.status, FdbTslStatus::Write);
        assert_eq!(tsl.time, 100);

        // Verify the PRE_WRITE TSL is read with time=0 and max_len
        let mut tsl2 = FdbTsl::default();
        tsl2.addr_index = next_idx_addr;
        db2.read_tsl(&flash, &mut tsl2);
        assert_eq!(tsl2.status, FdbTslStatus::PreWrite);
        assert_eq!(tsl2.time, 0, "PRE_WRITE TSL must have time=0");
        assert_eq!(tsl2.log_len, 256, "PRE_WRITE TSL log_len must be max_len");

        // Verify we can still append after recovery
        let mut data2 = [0xAAu8; 16];
        let mut blob2 = blob_make(&mut data2);
        blob2.size = 16;
        let result = db2.tsl_append_with_ts(&mut flash, &blob2, 200);
        assert!(result.is_ok(), "append after recovery must succeed: {:?}", result);
    }

    #[test]
    fn test_end_info_save() {
        // c: fdb_tsdb.c:396-407 — verify end_info is saved when sector closes
        let mut flash = MockFlash::new("test", 512, 1024, 512); // 2 sectors
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 512;
        db.parent.max_size = 1024;
        db.init(&mut flash, "test", "part", test_get_time, 64).unwrap();

        // Fill sector 0 to trigger sector switch
        let tsls_per_sec = (512 - SECTOR_HDR_DATA_SIZE) / (LOG_IDX_DATA_SIZE + wg_align(16) as usize);
        for i in 1..=tsls_per_sec {
            let mut data = [i as u8; 16];
            let mut blob = blob_make(&mut data);
            blob.size = 16;
            db.tsl_append_with_ts(&mut flash, &blob, i as FdbTime * 10).unwrap();
        }
        // Next append triggers sector switch (sector 0 → sector 1)
        let mut data = [0xFFu8; 16];
        let mut blob = blob_make(&mut data);
        blob.size = 16;
        db.tsl_append_with_ts(&mut flash, &blob, (tsls_per_sec + 1) as FdbTime * 10).unwrap();

        // Read sector 0 header from flash to verify end_info
        let mut hdr_buf = [0u8; core::mem::size_of::<TsdbSectorHdrData>()];
        flash.read(0, &mut hdr_buf).unwrap();

        // Check end_info[0] or end_info[1] has WRITE status
        let end0_status_idx = get_status(
            &hdr_buf[SECTOR_END0_STATUS_OFFSET..SECTOR_END0_STATUS_OFFSET + TSL_STATUS_TABLE_SIZE],
            FDB_TSL_STATUS_NUM as usize,
        );
        let end1_status_idx = get_status(
            &hdr_buf[SECTOR_END1_STATUS_OFFSET..SECTOR_END1_STATUS_OFFSET + TSL_STATUS_TABLE_SIZE],
            FDB_TSL_STATUS_NUM as usize,
        );

        let (end_time, end_idx, found) = if tsl_status_from_index(end0_status_idx) == FdbTslStatus::Write {
            (read_time_ne(&hdr_buf, SECTOR_END0_TIME_OFFSET),
             read_u32_ne(&hdr_buf, SECTOR_END0_IDX_OFFSET), true)
        } else if tsl_status_from_index(end1_status_idx) == FdbTslStatus::Write {
            (read_time_ne(&hdr_buf, SECTOR_END1_TIME_OFFSET),
             read_u32_ne(&hdr_buf, SECTOR_END1_IDX_OFFSET), true)
        } else {
            (0, 0, false)
        };

        assert!(found, "at least one end_info must have WRITE status after sector close");
        assert_eq!(
            end_time,
            tsls_per_sec as FdbTime * 10,
            "end_time must be the last TSL's timestamp before sector close"
        );
        // end_idx is the index of the last TSL in sector 0
        let expected_end_idx = SECTOR_HDR_DATA_SIZE as u32 + (tsls_per_sec as u32 - 1) * LOG_IDX_DATA_SIZE as u32;
        assert_eq!(
            end_idx, expected_end_idx,
            "end_idx must point to the last TSL in sector 0"
        );
    }

    #[test]
    fn test_reboot_persistence() {
        // c: fdb_tsdb_tc.c:110-114 — reboot (deinit + init) preserves data
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);

        // First boot: init + append 5 TSLs
        {
            let mut db = FdbTsdb::default();
            db.parent.sec_size = 4096;
            db.parent.max_size = 16384;
            db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();
            for i in 1..=5 {
                let mut data = [i as u8; 32];
                let mut blob = blob_make(&mut data);
                blob.size = 32;
                db.tsl_append_with_ts(&mut flash, &blob, i as FdbTime * 100).unwrap();
            }
            assert_eq!(db.last_time, 500);
            db.deinit().unwrap();
        }

        // Reboot: re-init and verify data persists
        {
            let mut db = FdbTsdb::default();
            db.parent.sec_size = 4096;
            db.parent.max_size = 16384;
            db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();
            assert_eq!(db.last_time, 500, "last_time must be recovered after reboot");

            let mut timestamps = Vec::new();
            db.tsl_iter(&flash, |tsl| {
                timestamps.push(tsl.time);
                false
            });
            assert_eq!(timestamps, vec![100, 200, 300, 400, 500], "all TSLs must persist after reboot");
        }
    }

    #[test]
    fn test_max_blob_count_boundary() {
        // c: fdb_tsdb.c:814-827 — max_blob_count with various configurations
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();

        // Verify max_blob_count is consistent with actual capacity
        let max_count = db.tsl_max_blob_count();
        assert!(max_count > 0);

        // Verify the formula: n_sec * (sec_size - hdr) / (idx_size + wg_align(max_len))
        let n_sec = 16384 / 4096;
        let avail_per_sec = 4096 - SECTOR_HDR_DATA_SIZE;
        let per_blob = LOG_IDX_DATA_SIZE + wg_align(256) as usize;
        assert_eq!(max_count, n_sec * (avail_per_sec / per_blob));

        // With small max_len, more TSLs fit
        let mut flash2 = MockFlash::new("test", 4096, 16384, 4096);
        let mut db2 = FdbTsdb::default();
        db2.parent.sec_size = 4096;
        db2.parent.max_size = 16384;
        db2.init(&mut flash2, "test", "part", test_get_time, 16).unwrap();
        let max_count_small = db2.tsl_max_blob_count();
        assert!(
            max_count_small > max_count,
            "smaller max_len should allow more TSLs"
        );
    }

    #[test]
    fn test_iter_empty_db() {
        // Iter on empty (freshly formatted) database should return no TSLs
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut db = FdbTsdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();

        let mut count = 0;
        db.tsl_iter(&flash, |_tsl| {
            count += 1;
            false
        });
        assert_eq!(count, 0, "empty DB iter must return 0 TSLs");

        db.tsl_iter_reverse(&flash, |_tsl| {
            count += 1;
            false
        });
        assert_eq!(count, 0, "empty DB reverse iter must return 0 TSLs");

        let qc = db.tsl_query_count(&flash, 0, 10000, FdbTslStatus::Write);
        assert_eq!(qc, 0, "empty DB query_count must return 0");
    }

    #[test]
    fn test_iter_by_time_no_match() {
        // Range query that doesn't match any TSL
        let (db, flash) = setup_tsdb_with_data(); // TSLs at 100, 200, 300, 400, 500

        let mut count = 0;
        db.tsl_iter_by_time(&flash, 1000, 2000, |_tsl| {
            count += 1;
            false
        });
        assert_eq!(count, 0, "range [1000,2000] must return 0 TSLs");
    }

    #[test]
    fn test_clean_after_reboot() {
        // c: fdb_tsdb_tc.c:211-226 — clean after reboot
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);

        // Boot 1: init + append data
        {
            let mut db = FdbTsdb::default();
            db.parent.sec_size = 4096;
            db.parent.max_size = 16384;
            db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();
            for i in 1..=5 {
                let mut data = [i as u8; 32];
                let mut blob = blob_make(&mut data);
                blob.size = 32;
                db.tsl_append_with_ts(&mut flash, &blob, i as FdbTime * 100).unwrap();
            }
            db.deinit().unwrap();
        }

        // Boot 2: clean
        {
            let mut db = FdbTsdb::default();
            db.parent.sec_size = 4096;
            db.parent.max_size = 16384;
            db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();
            db.tsl_clean(&mut flash);

            let mut count = 0;
            db.tsl_iter(&flash, |_tsl| {
                count += 1;
                false
            });
            assert_eq!(count, 0, "iter after clean must return 0 TSLs");
            assert_eq!(db.last_time, 0, "last_time must be 0 after clean");
            db.deinit().unwrap();
        }

        // Boot 3: verify still clean
        {
            let mut db = FdbTsdb::default();
            db.parent.sec_size = 4096;
            db.parent.max_size = 16384;
            db.init(&mut flash, "test", "part", test_get_time, 256).unwrap();
            let mut count = 0;
            db.tsl_iter(&flash, |_tsl| {
                count += 1;
                false
            });
            assert_eq!(count, 0, "iter after reboot+clean must still return 0 TSLs");
        }
    }
}
