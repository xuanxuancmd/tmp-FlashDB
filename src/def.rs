// c: fdb_def.h — All type definitions (enums, structs, type aliases)
//
// This module is the 1:1 Rust translation of fdb_def.h.
// On-flash structs (defined in .c files) will be added in Plan 2/3.

#![allow(dead_code)]

// ===== Software version (c: fdb_def.h:24-25) =====
// (defined in lib.rs)

// ===== Configuration constants (c: fdb_def.h:28-57, fdb_cfg_template.h) =====

/// c: fdb_def.h:29 — KV max name length
pub const FDB_KV_NAME_MAX: usize = 64;

/// c: fdb_def.h:34 — KV cache table size
#[cfg(feature = "kv_cache")]
pub const FDB_KV_CACHE_TABLE_SIZE: usize = 64;

/// c: fdb_def.h:39 — sector cache table size
#[cfg(feature = "kv_cache")]
pub const FDB_SECTOR_CACHE_TABLE_SIZE: usize = 8;

/// c: fdb_def.h:53 — file cache table size
#[cfg(feature = "file_mode")]
pub const FDB_FILE_CACHE_TABLE_SIZE: usize = 2;

// ===== Write granularity (c: fdb_def.h:55-57, fdb_cfg_template.h:38) =====
/// c: fdb_cfg_template.h:38 — Flash write granularity in bits.
/// 1=NOR flash, 8=STM32F2/F4, 32=STM32F1, 64=STM32F7, 128=STM32H5, 256=STM32H7.
//
// The `gran_*` Cargo features mirror the C `#if (FDB_WRITE_GRAN == N)` macro and
// are mutually exclusive in normal use (only one should be enabled at a time).
// Cargo's additive feature model cannot express mutual exclusion, so when
// multiple `gran_*` features are enabled simultaneously (e.g. via
// `cargo build --all-features`) the largest granularity wins. This is a harmless
// fallback that keeps `--all-features` compilable without changing on-flash
// layout for any single-feature configuration.
#[cfg(feature = "gran_256")]
pub const FDB_WRITE_GRAN: u32 = 256;

#[cfg(all(feature = "gran_128", not(feature = "gran_256")))]
pub const FDB_WRITE_GRAN: u32 = 128;

#[cfg(all(
    feature = "gran_64",
    not(any(feature = "gran_128", feature = "gran_256"))
))]
pub const FDB_WRITE_GRAN: u32 = 64;

#[cfg(all(
    feature = "gran_32",
    not(any(feature = "gran_64", feature = "gran_128", feature = "gran_256"))
))]
pub const FDB_WRITE_GRAN: u32 = 32;

#[cfg(all(
    feature = "gran_8",
    not(any(
        feature = "gran_32",
        feature = "gran_64",
        feature = "gran_128",
        feature = "gran_256"
    ))
))]
pub const FDB_WRITE_GRAN: u32 = 8;

#[cfg(not(any(
    feature = "gran_8",
    feature = "gran_32",
    feature = "gran_64",
    feature = "gran_128",
    feature = "gran_256"
)))]
pub const FDB_WRITE_GRAN: u32 = 1;

// ===== Byte constants (c: fdb_low_lvl.h:25-27) =====
/// c: fdb_low_lvl.h:25 — erased flash byte
pub const FDB_BYTE_ERASED: u8 = 0xFF;
/// c: fdb_low_lvl.h:27 — written flash byte
pub const FDB_BYTE_WRITTEN: u8 = 0x00;

/// c: fdb_low_lvl.h:48 — unused data marker (erased flash = 0xFFFFFFFF)
pub const FDB_DATA_UNUSED: u32 = 0xFFFF_FFFF;

/// c: fdb_low_lvl.h:54 — invalid address
pub const FDB_FAILED_ADDR: u32 = 0xFFFF_FFFF;

// ===== Status table size constants (c: fdb_low_lvl.h:43-44) =====
/// c: fdb_low_lvl.h:43 — store status table size
pub const FDB_STORE_STATUS_TABLE_SIZE: usize =
    status_table_size_const(FDB_SECTOR_STORE_STATUS_NUM as usize, FDB_WRITE_GRAN);
/// c: fdb_low_lvl.h:44 — dirty status table size
pub const FDB_DIRTY_STATUS_TABLE_SIZE: usize =
    status_table_size_const(FDB_SECTOR_DIRTY_STATUS_NUM as usize, FDB_WRITE_GRAN);

/// c: fdb_low_lvl.h:18-22 — FDB_STATUS_TABLE_SIZE macro
const fn status_table_size_const(status_num: usize, gran: u32) -> usize {
    if gran == 1 {
        (status_num * gran as usize + 7) / 8
    } else {
        ((status_num - 1) * gran as usize + 7) / 8
    }
}

// ===== Control command constants (c: fdb_def.h:87-104) =====
// c: fdb_def.h:87-93 — KVDB control commands
pub const FDB_KVDB_CTRL_SET_SEC_SIZE: u32 = 0x00;
pub const FDB_KVDB_CTRL_GET_SEC_SIZE: u32 = 0x01;
pub const FDB_KVDB_CTRL_SET_LOCK: u32 = 0x02;
pub const FDB_KVDB_CTRL_SET_UNLOCK: u32 = 0x03;
pub const FDB_KVDB_CTRL_SET_FILE_MODE: u32 = 0x09;
pub const FDB_KVDB_CTRL_SET_MAX_SIZE: u32 = 0x0A;
pub const FDB_KVDB_CTRL_SET_NOT_FORMAT: u32 = 0x0B;

// c: fdb_def.h:95-104 — TSDB control commands
pub const FDB_TSDB_CTRL_SET_SEC_SIZE: u32 = 0x00;
pub const FDB_TSDB_CTRL_GET_SEC_SIZE: u32 = 0x01;
pub const FDB_TSDB_CTRL_SET_LOCK: u32 = 0x02;
pub const FDB_TSDB_CTRL_SET_UNLOCK: u32 = 0x03;
pub const FDB_TSDB_CTRL_SET_ROLLOVER: u32 = 0x04;
pub const FDB_TSDB_CTRL_GET_ROLLOVER: u32 = 0x05;
pub const FDB_TSDB_CTRL_GET_LAST_TIME: u32 = 0x06;
pub const FDB_TSDB_CTRL_SET_FILE_MODE: u32 = 0x09;
pub const FDB_TSDB_CTRL_SET_MAX_SIZE: u32 = 0x0A;
pub const FDB_TSDB_CTRL_SET_NOT_FORMAT: u32 = 0x0B;

// ===== Time type (c: fdb_def.h:106-110) =====
/// c: fdb_def.h:107-109 — fdb_time_t, i32 or i64 based on timestamp_64bit feature
#[cfg(not(feature = "timestamp_64bit"))]
pub type FdbTime = i32;
#[cfg(feature = "timestamp_64bit")]
pub type FdbTime = i64;

/// c: fdb_def.h:112 — fdb_get_time function pointer type
pub type FdbGetTime = fn() -> FdbTime;

// ===== Default KV (c: fdb_def.h:114-123) =====
/// c: fdb_def.h:114-118 — default KV node
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FdbDefaultKvNode {
    pub key: &'static str,
    pub value: &'static [u8],
    pub value_len: usize,
}

/// c: fdb_def.h:120-123 — default KV collection
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FdbDefaultKv {
    pub kvs: &'static [FdbDefaultKvNode],
}

// ===== Error code (c: fdb_def.h:126-136) =====
/// c: fdb_def.h:126-136 — fdb_err_t
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdbErr {
    NoErr = 0,
    EraseErr,
    ReadErr,
    WriteErr,
    PartNotFound,
    KvNameErr,
    KvNameExist,
    SavedFull,
    InitFailed,
}

// ===== KV status (c: fdb_def.h:138-147) =====
/// c: fdb_def.h:138-146 — fdb_kv_status_t
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdbKvStatus {
    Unused = 0,
    PreWrite,
    Write,
    PreDelete,
    Deleted,
    ErrHdr,
}

/// c: fdb_def.h:145 — FDB_KV_STATUS_NUM
pub const FDB_KV_STATUS_NUM: u8 = 6;

// ===== TSL status (c: fdb_def.h:149-158) =====
/// c: fdb_def.h:149-157 — fdb_tsl_status_t
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdbTslStatus {
    Unused = 0,
    PreWrite,
    Write,
    UserStatus1,
    Deleted,
    UserStatus2,
}

/// c: fdb_def.h:156 — FDB_TSL_STATUS_NUM
pub const FDB_TSL_STATUS_NUM: u8 = 6;

// ===== KV node (c: fdb_def.h:161-174) =====
/// c: fdb_def.h:161-173 — key-value node runtime object
#[derive(Debug, Clone)]
pub struct FdbKv {
    pub status: FdbKvStatus,
    pub crc_is_ok: bool,
    pub name_len: u8,
    pub magic: u32,
    /// node total length (header + name + value), aligned by FDB_WRITE_GRAN
    pub len: u32,
    pub value_len: u32,
    /// c: fdb_def.h:168 — name buffer
    pub name: [u8; FDB_KV_NAME_MAX],
    /// c: fdb_def.h:169-172 — addr.start
    pub addr_start: u32,
    /// c: fdb_def.h:170 — addr.value
    pub addr_value: u32,
}

impl Default for FdbKv {
    fn default() -> Self {
        Self {
            status: FdbKvStatus::Unused,
            crc_is_ok: false,
            name_len: 0,
            magic: 0,
            len: 0,
            value_len: 0,
            name: [0; FDB_KV_NAME_MAX],
            addr_start: 0,
            addr_value: 0,
        }
    }
}

impl FdbKv {
    /// Get the KV name as a string slice
    pub fn name_str(&self) -> &str {
        let len = self.name_len as usize;
        if len <= FDB_KV_NAME_MAX {
            core::str::from_utf8(&self.name[..len]).unwrap_or("")
        } else {
            ""
        }
    }
}

// ===== KV iterator (c: fdb_def.h:176-184) =====
/// c: fdb_def.h:176-183 — fdb_kv_iterator
#[derive(Debug, Clone)]
pub struct FdbKvIterator {
    /// c: fdb_def.h:177 — current KV
    pub curr_kv: FdbKv,
    /// c: fdb_def.h:178 — iterated count
    pub iterated_cnt: u32,
    /// c: fdb_def.h:179 — total storage size iterated
    pub iterated_obj_bytes: usize,
    /// c: fdb_def.h:180 — total value size iterated
    pub iterated_value_bytes: usize,
    /// c: fdb_def.h:181 — current sector address
    pub sector_addr: u32,
    /// c: fdb_def.h:182 — traversed sector total length
    pub traversed_len: u32,
}

impl Default for FdbKvIterator {
    fn default() -> Self {
        Self {
            curr_kv: FdbKv::default(),
            iterated_cnt: 0,
            iterated_obj_bytes: 0,
            iterated_value_bytes: 0,
            sector_addr: 0,
            traversed_len: 0,
        }
    }
}

// ===== TSL node (c: fdb_def.h:187-196) =====
/// c: fdb_def.h:187-195 — time series log node runtime object
#[derive(Debug, Clone, Copy)]
pub struct FdbTsl {
    pub status: FdbTslStatus,
    pub time: FdbTime,
    /// log length, aligned by FDB_WRITE_GRAN
    pub log_len: u32,
    /// c: fdb_def.h:192 — addr.index
    pub addr_index: u32,
    /// c: fdb_def.h:193 — addr.log
    pub addr_log: u32,
}

impl Default for FdbTsl {
    fn default() -> Self {
        Self {
            status: FdbTslStatus::Unused,
            time: 0,
            log_len: 0,
            addr_index: 0,
            addr_log: 0,
        }
    }
}

// ===== Database type (c: fdb_def.h:199-202) =====
/// c: fdb_def.h:199-202 — fdb_db_type
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdbDbType {
    Kv = 0,
    Ts,
}

// ===== Sector store status (c: fdb_def.h:205-212) =====
/// c: fdb_def.h:205-211 — fdb_sector_store_status_t
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdbSectorStoreStatus {
    Unused = 0,
    Empty,
    Using,
    Full,
}

/// c: fdb_def.h:210 — FDB_SECTOR_STORE_STATUS_NUM
pub const FDB_SECTOR_STORE_STATUS_NUM: u8 = 4;

// ===== Sector dirty status (c: fdb_def.h:215-222) =====
/// c: fdb_def.h:215-221 — fdb_sector_dirty_status_t
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdbSectorDirtyStatus {
    Unused = 0,
    False,
    True,
    Gc,
}

/// c: fdb_def.h:220 — FDB_SECTOR_DIRTY_STATUS_NUM
pub const FDB_SECTOR_DIRTY_STATUS_NUM: u8 = 4;

// ===== KVDB sector info (c: fdb_def.h:225-237) =====
/// c: fdb_def.h:225-236 — kvdb_sec_info (runtime)
#[derive(Debug, Clone, Copy)]
pub struct KvdbSecInfo {
    pub check_ok: bool,
    pub store: FdbSectorStoreStatus,
    pub dirty: FdbSectorDirtyStatus,
    pub addr: u32,
    pub magic: u32,
    /// 0xFFFFFFFF: not combined
    pub combined: u32,
    pub remain: usize,
    pub empty_kv: u32,
}

impl Default for KvdbSecInfo {
    fn default() -> Self {
        Self {
            check_ok: false,
            store: FdbSectorStoreStatus::Unused,
            dirty: FdbSectorDirtyStatus::Unused,
            addr: 0,
            magic: 0,
            combined: FDB_DATA_UNUSED,
            remain: 0,
            empty_kv: 0,
        }
    }
}

// ===== TSDB sector info (c: fdb_def.h:240-253) =====
/// c: fdb_def.h:240-252 — tsdb_sec_info (runtime)
#[derive(Debug, Clone)]
pub struct TsdbSecInfo {
    pub check_ok: bool,
    pub status: FdbSectorStoreStatus,
    pub addr: u32,
    pub magic: u32,
    pub start_time: FdbTime,
    pub end_time: FdbTime,
    pub end_idx: u32,
    /// c: fdb_def.h:248 — end_info_stat[2]
    pub end_info_stat: [FdbTslStatus; 2],
    pub remain: usize,
    pub empty_idx: u32,
    pub empty_data: u32,
}

impl Default for TsdbSecInfo {
    fn default() -> Self {
        Self {
            check_ok: false,
            status: FdbSectorStoreStatus::Unused,
            addr: 0,
            magic: 0,
            start_time: FDB_DATA_UNUSED as FdbTime,
            end_time: FDB_DATA_UNUSED as FdbTime,
            end_idx: FDB_FAILED_ADDR,
            end_info_stat: [FdbTslStatus::Unused; 2],
            remain: 0,
            empty_idx: 0,
            empty_data: 0,
        }
    }
}

// ===== KV cache node (c: fdb_def.h:255-260) =====
/// c: fdb_def.h:255-259 — kv_cache_node
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KvCacheNode {
    /// c: fdb_def.h:256 — KV name's CRC32 low 16bit
    pub name_crc: u16,
    /// c: fdb_def.h:257 — KV node access active degree
    pub active: u16,
    /// c: fdb_def.h:258 — KV node address
    pub addr: u32,
}

impl Default for KvCacheNode {
    fn default() -> Self {
        Self {
            name_crc: 0,
            active: 0,
            addr: FDB_FAILED_ADDR,
        }
    }
}

// ===== Database structure (c: fdb_def.h:264-294) =====
/// c: fdb_def.h:264-294 — fdb_db (runtime base database structure)
#[derive(Debug)]
pub struct FdbDb {
    /// c: fdb_def.h:265 — database name
    pub name: &'static str,
    /// c: fdb.c:31 — partition/directory path identifier (FAL partition name or
    /// file directory in C). Stored so `db_path` can return it without the C
    /// `storage` union (which is replaced by the `FlashDevice` trait).
    pub path: &'static str,
    /// c: fdb_def.h:266 — database type
    pub type_: FdbDbType,
    /// c: fdb_def.h:275 — flash section size (multiple of block size)
    pub sec_size: u32,
    /// c: fdb_def.h:276 — database max size (multiple of section size)
    pub max_size: u32,
    /// c: fdb_def.h:277 — oldest sector start address
    pub oldest_addr: u32,
    /// c: fdb_def.h:278 — initialized successfully
    pub init_ok: bool,
    /// c: fdb_def.h:279 — is file mode
    pub file_mode: bool,
    /// c: fdb_def.h:280 — is NOT formatable mode
    pub not_formatable: bool,
    /// c: fdb_def.h:290 — lock callback
    pub lock: Option<fn(&mut FdbDb)>,
    /// c: fdb_def.h:291 — unlock callback
    pub unlock: Option<fn(&mut FdbDb)>,
}

impl Default for FdbDb {
    fn default() -> Self {
        Self {
            name: "",
            path: "",
            type_: FdbDbType::Kv,
            sec_size: 0,
            max_size: 0,
            oldest_addr: FDB_FAILED_ADDR,
            init_ok: false,
            file_mode: false,
            not_formatable: false,
            lock: None,
            unlock: None,
        }
    }
}

// ===== KVDB structure (c: fdb_def.h:297-319) =====
/// c: fdb_def.h:297-318 — fdb_kvdb (runtime)
///
/// C inheritance `struct fdb_kvdb { struct fdb_db parent; }` →
/// Rust composition + `AsRef<FdbDb>` (NOT Deref, per skill §3).
#[derive(Debug)]
pub struct FdbKvdb {
    /// c: fdb_def.h:298 — inherited from fdb_db
    pub parent: FdbDb,
    /// c: fdb_def.h:299 — default KV
    pub default_kvs: FdbDefaultKv,
    /// c: fdb_def.h:300 — GC check request
    pub gc_request: bool,
    /// c: fdb_def.h:301 — in recovery check on first reboot
    pub in_recovery_check: bool,
    /// c: fdb_def.h:302 — current KV
    pub cur_kv: FdbKv,
    /// c: fdb_def.h:303 — current sector
    pub cur_sector: KvdbSecInfo,
    /// c: fdb_def.h:304 — last operation was complete delete
    pub last_is_complete_del: bool,
    /// c: fdb_def.h:308 — KV cache table (feature-gated)
    #[cfg(feature = "kv_cache")]
    pub kv_cache_table: [KvCacheNode; FDB_KV_CACHE_TABLE_SIZE],
    /// c: fdb_def.h:310 — sector cache table (feature-gated)
    #[cfg(feature = "kv_cache")]
    pub sector_cache_table: [KvdbSecInfo; FDB_SECTOR_CACHE_TABLE_SIZE],
    /// c: fdb_def.h:314 — version number for auto update (feature-gated)
    #[cfg(feature = "kv_auto_update")]
    pub ver_num: u32,
}

impl Default for FdbKvdb {
    fn default() -> Self {
        Self {
            parent: FdbDb::default(),
            default_kvs: FdbDefaultKv { kvs: &[] },
            gc_request: false,
            in_recovery_check: false,
            cur_kv: FdbKv::default(),
            cur_sector: KvdbSecInfo::default(),
            last_is_complete_del: false,
            #[cfg(feature = "kv_cache")]
            kv_cache_table: [KvCacheNode::default(); FDB_KV_CACHE_TABLE_SIZE],
            #[cfg(feature = "kv_cache")]
            sector_cache_table: [KvdbSecInfo::default(); FDB_SECTOR_CACHE_TABLE_SIZE],
            #[cfg(feature = "kv_auto_update")]
            ver_num: 0,
        }
    }
}

impl AsRef<FdbDb> for FdbKvdb {
    fn as_ref(&self) -> &FdbDb {
        &self.parent
    }
}

// ===== TSDB structure (c: fdb_def.h:322-332) =====
/// c: fdb_def.h:322-331 — fdb_tsdb (runtime)
///
/// C inheritance `struct fdb_tsdb { struct fdb_db parent; }` →
/// Rust composition + `AsRef<FdbDb>`.
#[derive(Debug)]
pub struct FdbTsdb {
    /// c: fdb_def.h:323 — inherited from fdb_db
    pub parent: FdbDb,
    /// c: fdb_def.h:324 — current using sector
    pub cur_sec: TsdbSecInfo,
    /// c: fdb_def.h:325 — last TSL timestamp
    pub last_time: FdbTime,
    /// c: fdb_def.h:326 — current timestamp get function
    pub get_time: FdbGetTime,
    /// c: fdb_def.h:327 — maximum length of each log
    pub max_len: usize,
    /// c: fdb_def.h:328 — oldest data will rollover
    pub rollover: bool,
}

impl Default for FdbTsdb {
    fn default() -> Self {
        Self {
            parent: FdbDb::default(),
            cur_sec: TsdbSecInfo::default(),
            last_time: 0,
            get_time: || 0,
            max_len: 0,
            rollover: true,
        }
    }
}

impl AsRef<FdbDb> for FdbTsdb {
    fn as_ref(&self) -> &FdbDb {
        &self.parent
    }
}

// ===== Blob structure (c: fdb_def.h:335-344) =====
/// c: fdb_def.h:335-343 — fdb_blob (runtime)
///
/// Generic data transfer container used in KVDB/TSDB.
/// `buf` is a mutable borrow of an external buffer.
pub struct FdbBlob<'a> {
    /// c: fdb_def.h:336 — blob data buffer
    pub buf: &'a mut [u8],
    /// c: fdb_def.h:337 — blob data buffer size
    pub size: usize,
    /// c: fdb_def.h:339 — saved KV or TSL index address
    pub saved_meta_addr: u32,
    /// c: fdb_def.h:340 — blob data saved address
    pub saved_addr: u32,
    /// c: fdb_def.h:341 — blob data saved length
    pub saved_len: usize,
}

// ===== Layout assertions for on-flash-compatible types =====
// On-flash structs (SectorHdrData, KvHdrData, etc.) are defined in Plan 2/3.
// Here we verify the cache node layout which may be serialized.
const _: () = assert!(core::mem::size_of::<KvCacheNode>() == 8); // u16 + u16 + u32
