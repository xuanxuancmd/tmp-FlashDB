// c: fdb_kvdb.c — Key-Value Database feature
//
// 1:1 Rust translation of fdb_kvdb.c (1944 lines). The C `_fdb_flash_*` helpers
// dispatched on `db->file_mode` to FAL/file backends; in Rust the flash backend
// is supplied via the `FlashDevice` trait, so every flash-doing function takes an
// explicit `flash: &F` / `flash: &mut F` parameter (`F: FlashDevice`). `FdbKvdb`
// does NOT own the flash device (the Foundation removed C's `union storage`).
//
// On-flash structs (SectorHdrData, KvHdrData) use `#[repr(C)]` and are
// (de)serialised field-by-field via little-endian byte buffers — no `transmute`,
// no `unsafe` (see skill references/type-punning.md and offsetof.md).

#![allow(dead_code)]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::def::{
    FdbBlob, FdbDb, FdbDbType, FdbDefaultKv, FdbErr, FdbKv, FdbKvIterator,
    FdbKvStatus, FdbKvdb, FdbSectorDirtyStatus, FdbSectorStoreStatus, KvdbSecInfo, FDB_BYTE_ERASED,
    FDB_DATA_UNUSED, FDB_DIRTY_STATUS_TABLE_SIZE, FDB_FAILED_ADDR, FDB_KV_NAME_MAX,
    FDB_KV_STATUS_NUM, FDB_SECTOR_DIRTY_STATUS_NUM, FDB_SECTOR_STORE_STATUS_NUM,
    FDB_STORE_STATUS_TABLE_SIZE,
};
use crate::flash_trait::FlashDevice;
use crate::init::{deinit, init_ex, init_finish};
use crate::low_lvl::{
    align_down, blob_make, calc_crc32, continue_ff_addr, flash_erase, flash_read,
    flash_write, flash_write_align, get_status, read_status, set_status, status_table_size,
    wg_align, write_status,
};

// ===== Magic words & constants (c: fdb_kvdb.c:34-54) =====

/// c: fdb_kvdb.c:35 — magic word(`F`, `D`, `B`, `1`)
pub const SECTOR_MAGIC_WORD: u32 = 0x3042_4446;
/// c: fdb_kvdb.c:37 — magic word(`K`, `V`, `0`, `0`)
pub const KV_MAGIC_WORD: u32 = 0x3030_564B;
/// c: fdb_kvdb.c:39 — GC minimum number of empty sectors
pub const GC_MIN_EMPTY_SEC_NUM: u32 = 1;

/// c: fdb_kvdb.c:43 — the sector remain threshold before full status
pub const FDB_SEC_REMAIN_THRESHOLD: u32 = KV_HDR_DATA_SIZE + FDB_KV_NAME_MAX as u32;

/// c: fdb_kvdb.c:48 — the total remain empty sector threshold before GC
pub const FDB_GC_EMPTY_SEC_THRESHOLD: u32 = 1;

/// c: fdb_kvdb.c:53 — the string KV value buffer size for legacy fdb_get_kv
pub const FDB_STR_KV_VALUE_MAX_SIZE: usize = 128;

/// c: fdb_kvdb.c:62 — the sector is not combined value (FDB_BYTE_ERASED == 0xFF)
pub const SECTOR_NOT_COMBINED: u32 = 0xFFFF_FFFF;
/// c: fdb_kvdb.c:63 — the sector is combined value
pub const SECTOR_COMBINED: u32 = 0x0000_0000;

/// c: fdb_kvdb.c:71 — KV_STATUS_TABLE_SIZE macro
pub const KV_STATUS_TABLE_SIZE: usize = status_table_size(FDB_KV_STATUS_NUM as u32) as usize;

/// c: fdb_kvdb.c:100 — version number KV name (kv_auto_update feature)
pub const VER_NUM_KV_NAME: &str = "__ver_num__";

// ===== On-flash struct sizes (c: fdb_kvdb.c:75,79) =====

/// c: fdb_kvdb.c:75 — SECTOR_HDR_DATA_SIZE = FDB_WG_ALIGN(sizeof(struct sector_hdr_data))
pub const SECTOR_HDR_DATA_SIZE: u32 = wg_align(core::mem::size_of::<SectorHdrData>() as u32);
/// c: fdb_kvdb.c:79 — KV_HDR_DATA_SIZE = FDB_WG_ALIGN(sizeof(struct kv_hdr_data))
pub const KV_HDR_DATA_SIZE: u32 = wg_align(core::mem::size_of::<KvHdrData>() as u32);

// ===== On-flash struct definitions (c: fdb_kvdb.c:102-133) =====

/// c: fdb_kvdb.c:102-115 — sector header on-flash data
///
/// `#[repr(C)]` matches the C layout exactly, including alignment padding between
/// the status table and `magic`, and the conditional `padding` field for
/// FDB_WRITE_GRAN == 64/128/256 (mirrored via `#[cfg(feature = ...)]`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SectorHdrData {
    /// c: fdb_kvdb.c:103-106 — store/dirty status tables
    pub status_table: SectorStatusTable,
    /// c: fdb_kvdb.c:107 — magic word
    pub magic: u32,
    /// c: fdb_kvdb.c:108 — combined next sector number (default: not combined)
    pub combined: u32,
    /// c: fdb_kvdb.c:109 — reserved
    pub reserved: u32,
    /// c: fdb_kvdb.c:110-114 — align padding for 64bit and 128bit write granularity
    #[cfg(all(
        any(feature = "gran_64", feature = "gran_128"),
        not(feature = "gran_256")
    ))]
    pub padding: [u8; 4],
    /// c: fdb_kvdb.c:112-113 — align padding for 256bit write granularity
    #[cfg(feature = "gran_256")]
    pub padding: [u8; 20],
}

/// c: fdb_kvdb.c:103-106 — nested sector status table (store + dirty)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SectorStatusTable {
    /// c: fdb_kvdb.c:104 — sector store status
    pub store: [u8; FDB_STORE_STATUS_TABLE_SIZE],
    /// c: fdb_kvdb.c:105 — sector dirty status
    pub dirty: [u8; FDB_DIRTY_STATUS_TABLE_SIZE],
}

impl Default for SectorStatusTable {
    fn default() -> Self {
        Self {
            store: [FDB_BYTE_ERASED; FDB_STORE_STATUS_TABLE_SIZE],
            dirty: [FDB_BYTE_ERASED; FDB_DIRTY_STATUS_TABLE_SIZE],
        }
    }
}

/// c: fdb_kvdb.c:118-132 — KV header on-flash data
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KvHdrData {
    /// c: fdb_kvdb.c:119 — KV node status
    pub status_table: [u8; KV_STATUS_TABLE_SIZE],
    /// c: fdb_kvdb.c:120 — magic word(`K`, `V`, `0`, `0`)
    pub magic: u32,
    /// c: fdb_kvdb.c:121 — KV node total length (header + name + value), aligned
    pub len: u32,
    /// c: fdb_kvdb.c:122 — KV node crc32(name_len + data_len + name + value)
    pub crc32: u32,
    /// c: fdb_kvdb.c:123 — name length
    pub name_len: u8,
    /// c: fdb_kvdb.c:124 — value length
    pub value_len: u32,
    /// c: fdb_kvdb.c:125-126 — align padding for 64bit write granularity
    #[cfg(all(
        feature = "gran_64",
        not(any(feature = "gran_128", feature = "gran_256"))
    ))]
    pub padding: [u8; 4],
    /// c: fdb_kvdb.c:127-128 — align padding for 128bit write granularity
    #[cfg(all(feature = "gran_128", not(feature = "gran_256")))]
    pub padding: [u8; 12],
    /// c: fdb_kvdb.c:129-130 — align padding for 256bit write granularity
    #[cfg(feature = "gran_256")]
    pub padding: [u8; 15],
}

impl Default for KvHdrData {
    fn default() -> Self {
        Self {
            status_table: [FDB_BYTE_ERASED; KV_STATUS_TABLE_SIZE],
            magic: 0,
            len: 0,
            crc32: 0,
            name_len: 0,
            value_len: 0,
            #[cfg(all(
                feature = "gran_64",
                not(any(feature = "gran_128", feature = "gran_256"))
            ))]
            padding: [0; 4],
            #[cfg(all(feature = "gran_128", not(feature = "gran_256")))]
            padding: [0; 12],
            #[cfg(feature = "gran_256")]
            padding: [0; 15],
        }
    }
}

// ===== Field offsets (c: fdb_kvdb.c:76-82) =====
// C used `&((struct X *)0)->field` (null-pointer offsetof trick, UB in C).
// Rust uses the safe `core::mem::offset_of!` macro (stabilised 1.77).

/// c: fdb_kvdb.c:76 — SECTOR_STORE_OFFSET
pub const SECTOR_STORE_OFFSET: usize = core::mem::offset_of!(SectorHdrData, status_table.store);
/// c: fdb_kvdb.c:77 — SECTOR_DIRTY_OFFSET
pub const SECTOR_DIRTY_OFFSET: usize = core::mem::offset_of!(SectorHdrData, status_table.dirty);
/// c: fdb_kvdb.c:78 — SECTOR_MAGIC_OFFSET
pub const SECTOR_MAGIC_OFFSET: usize = core::mem::offset_of!(SectorHdrData, magic);
/// c: fdb_kvdb.c:80 — KV_MAGIC_OFFSET
pub const KV_MAGIC_OFFSET: usize = core::mem::offset_of!(KvHdrData, magic);
/// c: fdb_kvdb.c:81 — KV_LEN_OFFSET
pub const KV_LEN_OFFSET: usize = core::mem::offset_of!(KvHdrData, len);
/// c: fdb_kvdb.c:82 — KV_NAME_LEN_OFFSET
pub const KV_NAME_LEN_OFFSET: usize = core::mem::offset_of!(KvHdrData, name_len);

// ===== Compile-time layout assertions (match C sizeof for every GRAN) =====
// Each GRAN configuration produces a different struct size (conditional padding);
// verify all of them against the values computed from the C source.
//
// The cfg guards mirror the priority-nested `FDB_WRITE_GRAN` selection in
// `def.rs`: when multiple `gran_*` features are enabled simultaneously (e.g.
// `--all-features`) the largest granularity wins, so each assertion fires only
// when its corresponding granularity is the *active* one.

#[cfg(not(any(
    feature = "gran_8",
    feature = "gran_32",
    feature = "gran_64",
    feature = "gran_128",
    feature = "gran_256"
)))]
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == 16); // GRAN==1
#[cfg(not(any(
    feature = "gran_8",
    feature = "gran_32",
    feature = "gran_64",
    feature = "gran_128",
    feature = "gran_256"
)))]
const _: () = assert!(core::mem::size_of::<KvHdrData>() == 24); // GRAN==1

#[cfg(all(
    feature = "gran_8",
    not(any(
        feature = "gran_32",
        feature = "gran_64",
        feature = "gran_128",
        feature = "gran_256"
    ))
))]
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == 20);
#[cfg(all(
    feature = "gran_8",
    not(any(
        feature = "gran_32",
        feature = "gran_64",
        feature = "gran_128",
        feature = "gran_256"
    ))
))]
const _: () = assert!(core::mem::size_of::<KvHdrData>() == 28);

#[cfg(all(
    feature = "gran_32",
    not(any(feature = "gran_64", feature = "gran_128", feature = "gran_256"))
))]
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == 36);
#[cfg(all(
    feature = "gran_32",
    not(any(feature = "gran_64", feature = "gran_128", feature = "gran_256"))
))]
const _: () = assert!(core::mem::size_of::<KvHdrData>() == 40);

#[cfg(all(
    feature = "gran_64",
    not(any(feature = "gran_128", feature = "gran_256"))
))]
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == 64);
#[cfg(all(
    feature = "gran_64",
    not(any(feature = "gran_128", feature = "gran_256"))
))]
const _: () = assert!(core::mem::size_of::<KvHdrData>() == 64);

#[cfg(all(feature = "gran_128", not(feature = "gran_256")))]
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == 112);
#[cfg(all(feature = "gran_128", not(feature = "gran_256")))]
const _: () = assert!(core::mem::size_of::<KvHdrData>() == 112);

#[cfg(feature = "gran_256")]
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == 224);
#[cfg(feature = "gran_256")]
const _: () = assert!(core::mem::size_of::<KvHdrData>() == 196);

// ===== Safe little-endian byte helpers (no transmute) =====

#[inline]
fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    let mut tmp = [0u8; 4];
    tmp.copy_from_slice(&buf[offset..offset + 4]);
    u32::from_le_bytes(tmp)
}

#[inline]
fn write_u32_le(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
}

/// Size of SectorHdrData on flash (compile-time constant).
const SECTOR_HDR_SIZE: usize = core::mem::size_of::<SectorHdrData>();
/// Size of KvHdrData on flash (compile-time constant).
const KV_HDR_SIZE: usize = core::mem::size_of::<KvHdrData>();

impl SectorHdrData {
    /// Decode a sector header from a raw flash byte buffer (c: read via `(uint32_t*)&sec_hdr`).
    pub(crate) fn from_bytes(buf: &[u8; SECTOR_HDR_SIZE]) -> Self {
        let mut hdr = SectorHdrData::default();
        let store_end = SECTOR_STORE_OFFSET + FDB_STORE_STATUS_TABLE_SIZE;
        hdr.status_table.store.copy_from_slice(&buf[SECTOR_STORE_OFFSET..store_end]);
        let dirty_end = SECTOR_DIRTY_OFFSET + FDB_DIRTY_STATUS_TABLE_SIZE;
        hdr.status_table.dirty.copy_from_slice(&buf[SECTOR_DIRTY_OFFSET..dirty_end]);
        hdr.magic = read_u32_le(buf, SECTOR_MAGIC_OFFSET);
        hdr.combined = read_u32_le(buf, SECTOR_MAGIC_OFFSET + 4);
        hdr.reserved = read_u32_le(buf, SECTOR_MAGIC_OFFSET + 8);
        hdr
    }

    /// Encode a sector header into a raw flash byte buffer. The buffer is pre-filled
    /// with `FDB_BYTE_ERASED` to mirror C's `memset(&sec_hdr, FDB_BYTE_ERASED, sizeof)`.
    pub(crate) fn to_bytes(self) -> [u8; SECTOR_HDR_SIZE] {
        let mut buf = [FDB_BYTE_ERASED; SECTOR_HDR_SIZE];
        let store_end = SECTOR_STORE_OFFSET + FDB_STORE_STATUS_TABLE_SIZE;
        buf[SECTOR_STORE_OFFSET..store_end].copy_from_slice(&self.status_table.store);
        let dirty_end = SECTOR_DIRTY_OFFSET + FDB_DIRTY_STATUS_TABLE_SIZE;
        buf[SECTOR_DIRTY_OFFSET..dirty_end].copy_from_slice(&self.status_table.dirty);
        write_u32_le(&mut buf, SECTOR_MAGIC_OFFSET, self.magic);
        write_u32_le(&mut buf, SECTOR_MAGIC_OFFSET + 4, self.combined);
        write_u32_le(&mut buf, SECTOR_MAGIC_OFFSET + 8, self.reserved);
        // conditional padding stays FDB_BYTE_ERASED (matching C's memset)
        buf
    }
}

impl KvHdrData {
    /// Decode a KV header from a raw flash byte buffer (c: read via `(uint32_t*)&kv_hdr`).
    pub(crate) fn from_bytes(buf: &[u8; KV_HDR_SIZE]) -> Self {
        let mut hdr = KvHdrData::default();
        hdr.status_table.copy_from_slice(&buf[..KV_STATUS_TABLE_SIZE]);
        hdr.magic = read_u32_le(buf, KV_MAGIC_OFFSET);
        hdr.len = read_u32_le(buf, KV_LEN_OFFSET);
        hdr.crc32 = read_u32_le(buf, KV_LEN_OFFSET + 4);
        hdr.name_len = buf[KV_NAME_LEN_OFFSET];
        hdr.value_len = read_u32_le(buf, KV_NAME_LEN_OFFSET + 1);
        hdr
    }

    /// Encode a KV header into a raw flash byte buffer (pre-filled with `FDB_BYTE_ERASED`).
    pub(crate) fn to_bytes(self) -> [u8; KV_HDR_SIZE] {
        let mut buf = [FDB_BYTE_ERASED; KV_HDR_SIZE];
        buf[..KV_STATUS_TABLE_SIZE].copy_from_slice(&self.status_table);
        write_u32_le(&mut buf, KV_MAGIC_OFFSET, self.magic);
        write_u32_le(&mut buf, KV_LEN_OFFSET, self.len);
        write_u32_le(&mut buf, KV_LEN_OFFSET + 4, self.crc32);
        buf[KV_NAME_LEN_OFFSET] = self.name_len;
        write_u32_le(&mut buf, KV_NAME_LEN_OFFSET + 1, self.value_len);
        buf
    }
}

// ===== KV cache operations (c: fdb_kvdb.c:150-275, feature-gated) =====
// Only compiled when the `kv_cache` feature is enabled. The cache tables live on
// `FdbKvdb` (cfg-gated fields in def.rs). Cache functions clone the cached
// `KvdbSecInfo` rather than returning references, avoiding borrow entanglement.

#[cfg(feature = "kv_cache")]
impl FdbKvdb {
    /// c: fdb_kvdb.c:151-172 — update_sector_cache
    pub(crate) fn update_sector_cache(&mut self, sector: &KvdbSecInfo) {
        use crate::def::FDB_SECTOR_CACHE_TABLE_SIZE;
        let mut empty_index = FDB_SECTOR_CACHE_TABLE_SIZE;
        for i in 0..FDB_SECTOR_CACHE_TABLE_SIZE {
            if self.sector_cache_table[i].addr == sector.addr {
                if sector.check_ok {
                    self.sector_cache_table[i] = sector.clone();
                } else {
                    self.sector_cache_table[i].addr = FDB_DATA_UNUSED;
                }
                return;
            } else if self.sector_cache_table[i].addr == FDB_DATA_UNUSED {
                empty_index = i;
            }
        }
        if sector.check_ok && empty_index < FDB_SECTOR_CACHE_TABLE_SIZE {
            self.sector_cache_table[empty_index] = sector.clone();
        }
    }

    /// c: fdb_kvdb.c:177-188 — get_sector_from_cache (returns a clone on hit)
    pub(crate) fn get_sector_from_cache(&self, sec_addr: u32) -> Option<KvdbSecInfo> {
        use crate::def::FDB_SECTOR_CACHE_TABLE_SIZE;
        for i in 0..FDB_SECTOR_CACHE_TABLE_SIZE {
            if self.sector_cache_table[i].addr == sec_addr {
                return Some(self.sector_cache_table[i].clone());
            }
        }
        None
    }

    /// c: fdb_kvdb.c:190-197 — update_sector_empty_addr_cache
    pub(crate) fn update_sector_empty_addr_cache(&mut self, sec_addr: u32, empty_addr: u32) {
        use crate::def::FDB_SECTOR_CACHE_TABLE_SIZE;
        for i in 0..FDB_SECTOR_CACHE_TABLE_SIZE {
            if self.sector_cache_table[i].addr == sec_addr {
                self.sector_cache_table[i].empty_kv = empty_addr;
                self.sector_cache_table[i].remain =
                    self.parent.sec_size as usize - (empty_addr - sec_addr) as usize;
                return;
            }
        }
    }

    /// c: fdb_kvdb.c:199-205 — update_sector_status_store_cache
    pub(crate) fn update_sector_status_store_cache(
        &mut self,
        sec_addr: u32,
        status: FdbSectorStoreStatus,
    ) {
        use crate::def::FDB_SECTOR_CACHE_TABLE_SIZE;
        for i in 0..FDB_SECTOR_CACHE_TABLE_SIZE {
            if self.sector_cache_table[i].addr == sec_addr {
                self.sector_cache_table[i].store = status;
                return;
            }
        }
    }

    /// c: fdb_kvdb.c:207-246 — update_kv_cache (LRU-like)
    pub(crate) fn update_kv_cache(&mut self, name: &[u8], addr: u32) {
        use crate::def::{FDB_KV_CACHE_TABLE_SIZE, KvCacheNode};
        let name_crc = (calc_crc32(0, name) >> 16) as u16;
        let mut empty_index = FDB_KV_CACHE_TABLE_SIZE;
        let mut min_activity_index = FDB_KV_CACHE_TABLE_SIZE;
        let mut min_activity: u16 = 0xFFFF;
        for i in 0..FDB_KV_CACHE_TABLE_SIZE {
            if addr != FDB_DATA_UNUSED {
                if self.kv_cache_table[i].name_crc == name_crc {
                    self.kv_cache_table[i].addr = addr;
                    return;
                } else if self.kv_cache_table[i].addr == FDB_DATA_UNUSED
                    && empty_index == FDB_KV_CACHE_TABLE_SIZE
                {
                    empty_index = i;
                } else if self.kv_cache_table[i].addr != FDB_DATA_UNUSED {
                    if self.kv_cache_table[i].active > 0 {
                        self.kv_cache_table[i].active -= 1;
                    }
                    if self.kv_cache_table[i].active < min_activity {
                        min_activity_index = i;
                        min_activity = self.kv_cache_table[i].active;
                    }
                }
            } else if self.kv_cache_table[i].name_crc == name_crc {
                self.kv_cache_table[i].addr = FDB_DATA_UNUSED;
                self.kv_cache_table[i].active = 0;
                return;
            }
        }
        if empty_index < FDB_KV_CACHE_TABLE_SIZE {
            self.kv_cache_table[empty_index] = KvCacheNode {
                name_crc,
                active: FDB_KV_CACHE_TABLE_SIZE as u16,
                addr,
            };
        } else if min_activity_index < FDB_KV_CACHE_TABLE_SIZE {
            self.kv_cache_table[min_activity_index] = KvCacheNode {
                name_crc,
                active: FDB_KV_CACHE_TABLE_SIZE as u16,
                addr,
            };
        }
    }

    /// c: fdb_kvdb.c:251-274 — get_kv_from_cache (verifies name by reading flash)
    ///
    /// Takes `&mut self` to bump the matched entry's active counter (LRU).
    pub(crate) fn get_kv_from_cache<F: FlashDevice>(
        &mut self,
        flash: &F,
        name: &[u8],
    ) -> Option<u32> {
        use crate::def::FDB_KV_CACHE_TABLE_SIZE;
        let name_crc = (calc_crc32(0, name) >> 16) as u16;
        for i in 0..FDB_KV_CACHE_TABLE_SIZE {
            // copy the fields out so we don't hold a borrow while mutating active
            let (entry_addr, entry_name_crc, entry_active) = {
                let e = &self.kv_cache_table[i];
                (e.addr, e.name_crc, e.active)
            };
            if entry_addr != FDB_DATA_UNUSED && entry_name_crc == name_crc {
                let mut saved_name = [0u8; FDB_KV_NAME_MAX];
                let _ = flash_read(flash, entry_addr + KV_HDR_DATA_SIZE, &mut saved_name);
                if &saved_name[..name.len()] == name {
                    let new_active = if entry_active >= 0xFFFF - FDB_KV_CACHE_TABLE_SIZE as u16 {
                        0xFFFF
                    } else {
                        entry_active + FDB_KV_CACHE_TABLE_SIZE as u16
                    };
                    self.kv_cache_table[i].active = new_active;
                    return Some(entry_addr);
                }
            }
        }
        None
    }
}

// ===== Accessor helpers (c: fdb_kvdb.c:84-98 macros → methods) =====

impl FdbKvdb {
    /// c: fdb_kvdb.c:84 — db_name(db)
    #[inline]
    pub(crate) fn db_name(&self) -> &'static str {
        self.parent.name
    }
    /// c: fdb_kvdb.c:85 — db_init_ok(db)
    #[inline]
    pub(crate) fn db_init_ok(&self) -> bool {
        self.parent.init_ok
    }
    /// c: fdb_kvdb.c:86 — db_sec_size(db)
    #[inline]
    pub(crate) fn db_sec_size(&self) -> u32 {
        self.parent.sec_size
    }
    /// c: fdb_kvdb.c:87 — db_max_size(db)
    #[inline]
    pub(crate) fn db_max_size(&self) -> u32 {
        self.parent.max_size
    }
    /// c: fdb_kvdb.c:88 — db_oldest_addr(db)
    #[inline]
    pub(crate) fn db_oldest_addr(&self) -> u32 {
        self.parent.oldest_addr
    }
    /// c: fdb_kvdb.c:90-93 — db_lock(db)
    #[inline]
    pub(crate) fn db_lock(&mut self) {
        let lock = self.parent.lock;
        if let Some(lock) = lock {
            lock(&mut self.parent);
        }
    }
    /// c: fdb_kvdb.c:95-98 — db_unlock(db)
    #[inline]
    pub(crate) fn db_unlock(&mut self) {
        let unlock = self.parent.unlock;
        if let Some(unlock) = unlock {
            unlock(&mut self.parent);
        }
    }
    /// c: fdb_kvdb.c:73 — SECTOR_NUM macro (runtime, depends on db config)
    #[inline]
    pub(crate) fn sector_num(&self) -> u32 {
        self.db_max_size() / self.db_sec_size()
    }
}

// ===== Status index ↔ enum conversion helpers =====
// C casts the integer status index directly to the enum (`(fdb_kv_status_t) idx`).
// Rust enums need an explicit match (no unsafe transmute).

/// c: fdb_kvdb.c — cast to fdb_kv_status_t
fn kv_status_from_index(index: usize) -> FdbKvStatus {
    match index {
        0 => FdbKvStatus::Unused,
        1 => FdbKvStatus::PreWrite,
        2 => FdbKvStatus::Write,
        3 => FdbKvStatus::PreDelete,
        4 => FdbKvStatus::Deleted,
        _ => FdbKvStatus::ErrHdr,
    }
}

/// c: fdb_kvdb.c — cast to fdb_sector_store_status_t
fn sector_store_status_from_index(index: usize) -> FdbSectorStoreStatus {
    match index {
        0 => FdbSectorStoreStatus::Unused,
        1 => FdbSectorStoreStatus::Empty,
        2 => FdbSectorStoreStatus::Using,
        _ => FdbSectorStoreStatus::Full,
    }
}

/// c: fdb_kvdb.c — cast to fdb_sector_dirty_status_t
fn sector_dirty_status_from_index(index: usize) -> FdbSectorDirtyStatus {
    match index {
        0 => FdbSectorDirtyStatus::Unused,
        1 => FdbSectorDirtyStatus::False,
        2 => FdbSectorDirtyStatus::True,
        _ => FdbSectorDirtyStatus::Gc,
    }
}

/// c: fdb_kvdb.c:609-620 — fdb_is_str (is every byte a printable ASCII char?)
fn fdb_is_str(value: &[u8]) -> bool {
    // c: #define __is_print(ch) ((unsigned int)((ch) - ' ') < 127u - ' ')
    value
        .iter()
        .all(|&ch| (ch as u32).wrapping_sub(b' ' as u32) < (127u32 - b' ' as u32))
}

// ===== KV read / find / iterate (c: fdb_kvdb.c:280-644) =====
//
// All read-chain functions take `&mut F flash` because `read_kv` may write the
// `ERR_HDR` status to flash when it detects a corrupt KV length (c: fdb_kvdb.c:366).
// This mirrors C where every `fdb_kvdb_t db` is a mutable pointer.

impl FdbKvdb {
    /// c: fdb_kvdb.c:280-310 — find_next_kv_addr
    ///
    /// Scan flash for the next KV magic word. Returns the KV start address
    /// (magic position minus `KV_MAGIC_OFFSET`) or `FDB_FAILED_ADDR`.
    pub(crate) fn find_next_kv_addr<F: FlashDevice>(
        &self,
        flash: &F,
        start: u32,
        end: u32,
    ) -> u32 {
        let start_bak = start;

        #[cfg(feature = "kv_cache")]
        {
            let sec_addr = align_down(start, self.db_sec_size());
            if let Some(sec) = self.get_sector_from_cache(sec_addr) {
                if start == sec.empty_kv {
                    return FDB_FAILED_ADDR;
                }
            }
        }

        let mut buf = [0u8; 32];
        let mut cur = start;
        // c: for (; start < end && start + sizeof(buf) < end; start += sizeof(buf) - sizeof(uint32_t))
        while cur < end && cur + 32 < end {
            if flash_read(flash, cur, &mut buf).is_err() {
                return FDB_FAILED_ADDR;
            }
            // c: for (i = 0; i < sizeof(buf) - sizeof(uint32_t) && start + i < end; i++)
            for i in 0..28 {
                if cur + i as u32 >= end {
                    break;
                }
                // c: Little Endian Order — magic = buf[i] + (buf[i+1]<<8) + ...
                let magic = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
                if magic == KV_MAGIC_WORD
                    && cur + i as u32 - KV_MAGIC_OFFSET as u32 >= start_bak
                {
                    return cur + i as u32 - KV_MAGIC_OFFSET as u32;
                }
            }
            cur += 28; // sizeof(buf) - sizeof(uint32_t)
        }

        FDB_FAILED_ADDR
    }

    /// c: fdb_kvdb.c:312-346 — get_next_kv_addr
    ///
    /// Compute the next KV address within a sector, relative to `pre_kv`.
    pub(crate) fn get_next_kv_addr<F: FlashDevice>(
        &self,
        flash: &F,
        sector: &KvdbSecInfo,
        pre_kv: &FdbKv,
    ) -> u32 {
        if sector.store == FdbSectorStoreStatus::Empty {
            return FDB_FAILED_ADDR;
        }

        if pre_kv.addr_start == FDB_FAILED_ADDR {
            // the first KV address
            sector.addr + SECTOR_HDR_DATA_SIZE
        } else if pre_kv.addr_start <= sector.addr + self.db_sec_size() {
            let next = if pre_kv.crc_is_ok {
                pre_kv.addr_start + pre_kv.len
            } else {
                // when pre_kv CRC check failed, advance by 1 aligned unit
                pre_kv.addr_start + wg_align(1)
            };
            // check and find next KV address
            let found =
                self.find_next_kv_addr(flash, next, sector.addr + self.db_sec_size() - SECTOR_HDR_DATA_SIZE);
            if found == FDB_FAILED_ADDR
                || found > sector.addr + self.db_sec_size()
                || pre_kv.len == 0
            {
                // TODO Sector continuous mode
                return FDB_FAILED_ADDR;
            }
            found
        } else {
            // no KV
            FDB_FAILED_ADDR
        }
    }

    /// c: fdb_kvdb.c:348-414 — read_kv
    ///
    /// Read a KV node (header + name + value) and verify its CRC32. On a corrupt
    /// length the KV is marked `ERR_HDR` on flash (matching C's self-healing).
    pub(crate) fn read_kv<F: FlashDevice>(
        &self,
        flash: &mut F,
        kv: &mut FdbKv,
    ) -> Result<(), FdbErr> {
        let mut hdr_buf = [0u8; KV_HDR_SIZE];
        // c: _fdb_flash_read(db, kv->addr.start, &kv_hdr, sizeof(struct kv_hdr_data))
        let _ = flash_read(flash, kv.addr_start, &mut hdr_buf);
        let kv_hdr = KvHdrData::from_bytes(&hdr_buf);

        kv.status = kv_status_from_index(get_status(
            &kv_hdr.status_table,
            FDB_KV_STATUS_NUM as usize,
        ));
        kv.len = kv_hdr.len;

        if kv.len == u32::MAX || kv.len > self.db_max_size() || kv.len < KV_HDR_DATA_SIZE {
            // the KV length was not write, so reserve the info for current KV
            kv.len = KV_HDR_DATA_SIZE;
            if kv.status != FdbKvStatus::ErrHdr {
                kv.status = FdbKvStatus::ErrHdr;
                let mut status_table = kv_hdr.status_table;
                // c: _fdb_write_status(..., FDB_KV_ERR_HDR, true)
                write_status(
                    flash,
                    kv.addr_start,
                    &mut status_table,
                    FDB_KV_STATUS_NUM as usize,
                    FdbKvStatus::ErrHdr as usize,
                )?;
            }
            kv.crc_is_ok = false;
            return Err(FdbErr::ReadErr);
        } else if kv.len > self.db_sec_size() - SECTOR_HDR_DATA_SIZE && kv.len < self.db_max_size() {
            // TODO Sector continuous mode, or the write length is not written completely
        }

        // CRC32 data: header.name_len(4) + header.value_len(4) + name + value
        let mut calc = 0u32;
        // c: fdb_calc_crc32(calc_crc32, &kv_hdr.name_len, sizeof(uint32_t))
        calc = calc_crc32(calc, &hdr_buf[KV_NAME_LEN_OFFSET..KV_NAME_LEN_OFFSET + 4]);
        let value_len_off = KV_NAME_LEN_OFFSET + 4; // == offset_of!(KvHdrData, value_len)
        // c: fdb_calc_crc32(calc_crc32, &kv_hdr.value_len, sizeof(uint32_t))
        calc = calc_crc32(calc, &hdr_buf[value_len_off..value_len_off + 4]);
        let crc_data_len = (kv.len - KV_HDR_DATA_SIZE) as usize;
        let mut buf = [0u8; 32];
        let mut len = 0usize;
        while len < crc_data_len {
            let size = if len + 32 < crc_data_len {
                32
            } else {
                crc_data_len - len
            };
            // c: read FDB_WG_ALIGN(size) bytes, CRC over `size` bytes
            let read_size = wg_align(size as u32) as usize;
            let _ = flash_read(
                flash,
                kv.addr_start + KV_HDR_DATA_SIZE + len as u32,
                &mut buf[..read_size],
            );
            calc = calc_crc32(calc, &buf[..size]);
            len += size;
        }

        if calc != kv_hdr.crc32 {
            // CRC check failed — try read the name (may itself have errors)
            let name_len = if kv_hdr.name_len as usize > FDB_KV_NAME_MAX {
                FDB_KV_NAME_MAX
            } else {
                kv_hdr.name_len as usize
            };
            kv.crc_is_ok = false;
            let read_len = wg_align(name_len as u32) as usize;
            let _ = flash_read(
                flash,
                kv.addr_start + KV_HDR_DATA_SIZE,
                &mut kv.name[..read_len],
            );
            Err(FdbErr::ReadErr)
        } else {
            kv.crc_is_ok = true;
            let kv_name_addr = kv.addr_start + KV_HDR_DATA_SIZE;
            let read_len = wg_align(kv_hdr.name_len as u32) as usize;
            // the name is behind the aligned KV header
            let _ = flash_read(flash, kv_name_addr, &mut kv.name[..read_len]);
            // the value is behind the aligned name
            kv.addr_value = kv_name_addr + read_len as u32;
            kv.value_len = kv_hdr.value_len;
            kv.name_len = kv_hdr.name_len;
            Ok(())
        }
    }

    /// c: fdb_kvdb.c:416-502 — read_sector_info
    ///
    /// Read a sector header and (when `traversal`) iterate KVs to compute the
    /// remaining space. Returns `Err(InitFailed)` when the magic/combined check
    /// fails, or `Err(ReadErr)` when a corrupt KV is encountered during traversal.
    pub(crate) fn read_sector_info<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        addr: u32,
        sector: &mut KvdbSecInfo,
        traversal: bool,
    ) -> Result<(), FdbErr> {
        // c: FDB_ASSERT(addr % db_sec_size(db) == 0)
        assert!(addr.is_multiple_of(self.db_sec_size()), "sector addr must be aligned");

        #[cfg(feature = "kv_cache")]
        {
            if let Some(sec_cache) = self.get_sector_from_cache(addr) {
                if !traversal || (traversal && sec_cache.empty_kv != FDB_FAILED_ADDR) {
                    *sector = sec_cache;
                    return Ok(());
                }
            }
        }

        // read sector header raw data
        let mut sec_hdr_buf = [0u8; SECTOR_HDR_SIZE];
        let _ = flash_read(flash, addr, &mut sec_hdr_buf);
        let sec_hdr = SectorHdrData::from_bytes(&sec_hdr_buf);

        sector.store = FdbSectorStoreStatus::Unused;
        sector.dirty = FdbSectorDirtyStatus::Unused;
        sector.addr = addr;
        sector.magic = sec_hdr.magic;
        // check magic word and combined value
        if sector.magic != SECTOR_MAGIC_WORD
            || (sec_hdr.combined != SECTOR_NOT_COMBINED && sec_hdr.combined != SECTOR_COMBINED)
        {
            sector.check_ok = false;
            sector.combined = SECTOR_NOT_COMBINED;
            return Err(FdbErr::InitFailed);
        }
        sector.check_ok = true;
        sector.combined = sec_hdr.combined;
        sector.store = sector_store_status_from_index(get_status(
            &sec_hdr.status_table.store,
            FDB_SECTOR_STORE_STATUS_NUM as usize,
        ));
        sector.dirty = sector_dirty_status_from_index(get_status(
            &sec_hdr.status_table.dirty,
            FDB_SECTOR_DIRTY_STATUS_NUM as usize,
        ));

        let mut result = Ok(());
        if traversal {
            sector.remain = 0;
            sector.empty_kv = sector.addr + SECTOR_HDR_DATA_SIZE;
            if sector.store == FdbSectorStoreStatus::Empty {
                sector.remain = (self.db_sec_size() - SECTOR_HDR_DATA_SIZE) as usize;
            } else if sector.store == FdbSectorStoreStatus::Using {
                let mut kv_obj = FdbKv::default();
                sector.remain = (self.db_sec_size() - SECTOR_HDR_DATA_SIZE) as usize;
                kv_obj.addr_start = sector.addr + SECTOR_HDR_DATA_SIZE;
                loop {
                    let _ = self.read_kv(flash, &mut kv_obj);
                    if !kv_obj.crc_is_ok
                        && kv_obj.status != FdbKvStatus::PreWrite
                        && kv_obj.status != FdbKvStatus::ErrHdr
                    {
                        sector.remain = 0;
                        result = Err(FdbErr::ReadErr);
                        break;
                    }
                    sector.empty_kv += kv_obj.len;
                    sector.remain -= kv_obj.len as usize;
                    let next = self.get_next_kv_addr(flash, sector, &kv_obj);
                    if next == FDB_FAILED_ADDR {
                        break;
                    }
                    kv_obj.addr_start = next;
                }
            }
            // check the empty KV address by reading continue 0xFF on flash
            let ff_addr = continue_ff_addr(
                flash,
                sector.empty_kv,
                sector.addr + self.db_sec_size(),
            );
            if sector.empty_kv != ff_addr {
                sector.empty_kv = ff_addr;
                sector.remain = (self.db_sec_size() - (ff_addr - sector.addr)) as usize;
            }

            #[cfg(feature = "kv_cache")]
            {
                self.update_sector_cache(sector);
            }
        } else {
            #[cfg(feature = "kv_cache")]
            {
                let in_cache = self.get_sector_from_cache(sector.addr).is_some();
                if !in_cache {
                    sector.empty_kv = FDB_FAILED_ADDR;
                    sector.remain = 0;
                    self.update_sector_cache(sector);
                }
            }
        }

        result
    }

    /// c: fdb_kvdb.c:504-526 — get_next_sector_addr
    pub(crate) fn get_next_sector_addr(
        &self,
        pre_sec: &KvdbSecInfo,
        traversed_len: u32,
    ) -> u32 {
        let cur_block_size = if pre_sec.combined == SECTOR_NOT_COMBINED {
            self.db_sec_size()
        } else {
            pre_sec.combined * self.db_sec_size()
        };

        if traversed_len + cur_block_size <= self.db_max_size() {
            if pre_sec.addr + cur_block_size < self.db_max_size() {
                pre_sec.addr + cur_block_size
            } else {
                // the next sector is on the top of the database
                0
            }
        } else {
            // finished
            FDB_FAILED_ADDR
        }
    }

    /// c: fdb_kvdb.c:528-557 — kv_iterator
    ///
    /// Generic KV iterator. The callback receives `(&F flash, &FdbKv kv)` and
    /// returns `true` to stop iteration (matching C's `callback(kv, arg1, arg2)`).
    pub(crate) fn kv_iterator<F, Cb>(&mut self, flash: &mut F, kv: &mut FdbKv, mut callback: Cb)
    where
        F: FlashDevice,
        Cb: FnMut(&F, &FdbKv) -> bool,
    {
        let mut sector = KvdbSecInfo::default();
        let mut sec_addr = self.db_oldest_addr();
        let mut traversed_len = 0u32;

        loop {
            traversed_len += self.db_sec_size();
            if self
                .read_sector_info(flash, sec_addr, &mut sector, false)
                .is_err()
            {
                // c: continue (sector header invalid)
            } else if sector.store == FdbSectorStoreStatus::Using
                || sector.store == FdbSectorStoreStatus::Full
            {
                // sector has KV
                kv.addr_start = sector.addr + SECTOR_HDR_DATA_SIZE;
                loop {
                    let _ = self.read_kv(flash, kv);
                    // iterator is interrupted when callback return true
                    if callback(flash, kv) {
                        return;
                    }
                    let next = self.get_next_kv_addr(flash, &sector, kv);
                    if next == FDB_FAILED_ADDR {
                        break;
                    }
                    kv.addr_start = next;
                }
            }
            sec_addr = self.get_next_sector_addr(&sector, traversed_len);
            if sec_addr == FDB_FAILED_ADDR {
                break;
            }
        }
    }

    /// c: fdb_kvdb.c:576-583 — find_kv_no_cache
    pub(crate) fn find_kv_no_cache<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        key: &[u8],
        kv: &mut FdbKv,
    ) -> bool {
        let mut find_ok = false;
        let key_len = key.len();
        // c: find_kv_cb inlined as a closure (void* arg1=key, arg2=&find_ok)
        self.kv_iterator(flash, kv, |_flash, cur_kv| {
            if key_len != cur_kv.name_len as usize {
                return false;
            }
            // check KV: crc_is_ok && status == WRITE && name matches
            if cur_kv.crc_is_ok
                && cur_kv.status == FdbKvStatus::Write
                && &cur_kv.name[..key_len] == key
            {
                find_ok = true;
                return true;
            }
            false
        });
        find_ok
    }

    /// c: fdb_kvdb.c:585-607 — find_kv (with optional cache)
    pub(crate) fn find_kv<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        key: &[u8],
        kv: &mut FdbKv,
    ) -> bool {
        #[cfg(feature = "kv_cache")]
        {
            if let Some(addr) = self.get_kv_from_cache(flash, key) {
                kv.addr_start = addr;
                let _ = self.read_kv(flash, kv);
                return true;
            }
        }

        let find_ok = self.find_kv_no_cache(flash, key, kv);

        #[cfg(feature = "kv_cache")]
        {
            if find_ok {
                self.update_kv_cache(key, kv.addr_start);
            }
        }

        find_ok
    }

    /// c: fdb_kvdb.c:622-644 — get_kv
    ///
    /// Find a KV by key and optionally read its value into `value_buf`.
    /// Returns the number of bytes read; `value_len_out` receives the full value
    /// length (even when the buffer is smaller).
    pub(crate) fn get_kv<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        key: &[u8],
        value_buf: &mut [u8],
        value_len_out: Option<&mut usize>,
    ) -> usize {
        let mut kv = FdbKv::default();
        let mut read_len = 0usize;
        let mut out_len: usize = 0;

        if self.find_kv(flash, key, &mut kv) {
            out_len = kv.value_len as usize;
            let buf_len = value_buf.len();
            // c: if (buf_len > kv.value_len) read_len = kv.value_len; else read_len = buf_len;
            read_len = if buf_len > kv.value_len as usize {
                kv.value_len as usize
            } else {
                buf_len
            };
            let _ = flash_read(flash, kv.addr_value, &mut value_buf[..read_len]);
        }
        // c: else if (value_len) *value_len = 0; — out_len stays 0 when not found
        if let Some(vl) = value_len_out {
            *vl = out_len;
        }

        read_len
    }
}

// ===== KV write / format / GC / public CRUD API (c: fdb_kvdb.c:755-1431) =====

/// c: fdb_kvdb.c:755-767 — write_kv_hdr
///
/// Write the KV header: status table (PRE_WRITE) then magic..end. Free function
/// (operates on the on-flash header, no db config needed).
fn write_kv_hdr<F: FlashDevice>(
    flash: &mut F,
    addr: u32,
    kv_hdr: &mut KvHdrData,
) -> Result<(), FdbErr> {
    // c: _fdb_write_status(db, addr, kv_hdr->status_table, KV_STATUS_NUM, PRE_WRITE, false)
    write_status(
        flash,
        addr,
        &mut kv_hdr.status_table,
        FDB_KV_STATUS_NUM as usize,
        FdbKvStatus::PreWrite as usize,
    )?;
    // c: _fdb_flash_write(db, addr + KV_MAGIC_OFFSET, &kv_hdr->magic, sizeof - KV_MAGIC_OFFSET, false)
    let buf = kv_hdr.to_bytes();
    flash_write(flash, addr + KV_MAGIC_OFFSET as u32, &buf[KV_MAGIC_OFFSET..])?;
    Ok(())
}

impl FdbKvdb {
    /// c: fdb_kvdb.c:769-827 — format_sector
    ///
    /// Erase a sector and write its header (magic, combined, store=Empty, dirty=False).
    pub(crate) fn format_sector<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        addr: u32,
        combined_value: u32,
    ) -> Result<(), FdbErr> {
        // c: FDB_ASSERT(addr % db_sec_size(db) == 0)
        assert!(addr.is_multiple_of(self.db_sec_size()), "sector addr must be aligned");

        flash_erase(flash, addr, self.db_sec_size())?;

        let mut sec_hdr = SectorHdrData::default(); // memset to FDB_BYTE_ERASED
        set_status(
            &mut sec_hdr.status_table.store,
            FDB_SECTOR_STORE_STATUS_NUM as usize,
            FdbSectorStoreStatus::Empty as usize,
        );
        set_status(
            &mut sec_hdr.status_table.dirty,
            FDB_SECTOR_DIRTY_STATUS_NUM as usize,
            FdbSectorDirtyStatus::False as usize,
        );
        sec_hdr.magic = SECTOR_MAGIC_WORD;
        sec_hdr.combined = combined_value;
        sec_hdr.reserved = FDB_DATA_UNUSED;

        #[cfg(not(any(
            feature = "gran_8",
            feature = "gran_32",
            feature = "gran_64",
            feature = "gran_128",
            feature = "gran_256"
        )))]
        {
            // c: GRAN==1 — write the whole header in one program
            let buf = sec_hdr.to_bytes();
            flash_write(flash, addr, &buf[..SECTOR_HDR_DATA_SIZE as usize])?;
        }
        #[cfg(any(
            feature = "gran_8",
            feature = "gran_32",
            feature = "gran_64",
            feature = "gran_128",
            feature = "gran_256"
        ))]
        {
            // c: GRAN>1 — separate programs to avoid re-program issues on STM32L4xx
            let mut store_table = sec_hdr.status_table.store;
            write_status(
                flash,
                addr + SECTOR_STORE_OFFSET as u32,
                &mut store_table,
                FDB_SECTOR_STORE_STATUS_NUM as usize,
                FdbSectorStoreStatus::Empty as usize,
            )?;
            let mut dirty_table = sec_hdr.status_table.dirty;
            write_status(
                flash,
                addr + SECTOR_DIRTY_OFFSET as u32,
                &mut dirty_table,
                FDB_SECTOR_DIRTY_STATUS_NUM as usize,
                FdbSectorDirtyStatus::False as usize,
            )?;
            let buf = sec_hdr.to_bytes();
            flash_write(
                flash,
                addr + SECTOR_MAGIC_OFFSET as u32,
                &buf[SECTOR_MAGIC_OFFSET..SECTOR_HDR_DATA_SIZE as usize],
            )?;
        }

        #[cfg(feature = "kv_cache")]
        {
            // c: delete the sector cache (check_ok=false → cache entry addr = FDB_DATA_UNUSED)
            let sector = KvdbSecInfo {
                addr,
                check_ok: false,
                empty_kv: FDB_FAILED_ADDR,
                ..KvdbSecInfo::default()
            };
            self.update_sector_cache(&sector);
        }

        Ok(())
    }

    /// c: fdb_kvdb.c:829-861 — update_sec_status
    ///
    /// Transition the sector store status (Empty→Using, Using→Full) based on remain.
    pub(crate) fn update_sec_status<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        sector: &mut KvdbSecInfo,
        new_kv_len: usize,
        mut is_full: Option<&mut bool>,
    ) -> Result<(), FdbErr> {
        let mut status_table = [0u8; FDB_STORE_STATUS_TABLE_SIZE];
        if sector.store == FdbSectorStoreStatus::Empty {
            // change the sector status to using
            write_status(
                flash,
                sector.addr,
                &mut status_table,
                FDB_SECTOR_STORE_STATUS_NUM as usize,
                FdbSectorStoreStatus::Using as usize,
            )?;
            #[cfg(feature = "kv_cache")]
            {
                self.update_sector_status_store_cache(sector.addr, FdbSectorStoreStatus::Using);
            }
        } else if sector.store == FdbSectorStoreStatus::Using {
            // check remain size
            if sector.remain < FDB_SEC_REMAIN_THRESHOLD as usize
                || sector.remain - new_kv_len < FDB_SEC_REMAIN_THRESHOLD as usize
            {
                // change the sector status to full
                write_status(
                    flash,
                    sector.addr,
                    &mut status_table,
                    FDB_SECTOR_STORE_STATUS_NUM as usize,
                    FdbSectorStoreStatus::Full as usize,
                )?;
                #[cfg(feature = "kv_cache")]
                {
                    self.update_sector_status_store_cache(sector.addr, FdbSectorStoreStatus::Full);
                }
                if let Some(full) = is_full.as_deref_mut() {
                    *full = true;
                }
            } else if let Some(full) = is_full {
                *full = false;
            }
        }
        Ok(())
    }

    /// c: fdb_kvdb.c:863-883 — sector_iterator
    ///
    /// Generic sector iterator. The callback receives `&KvdbSecInfo` and returns
    /// `true` to stop iteration. `status == Unused` matches all sectors.
    pub(crate) fn sector_iterator<F, Cb>(
        &mut self,
        flash: &mut F,
        sector: &mut KvdbSecInfo,
        status: FdbSectorStoreStatus,
        mut callback: Cb,
        traversal_kv: bool,
    ) where
        F: FlashDevice,
        Cb: FnMut(&KvdbSecInfo) -> bool,
    {
        let mut sec_addr = self.db_oldest_addr();
        let mut traversed_len = 0u32;
        loop {
            traversed_len += self.db_sec_size();
            let _ = self.read_sector_info(flash, sec_addr, sector, false);
            if status == FdbSectorStoreStatus::Unused || status == sector.store {
                if traversal_kv {
                    let _ = self.read_sector_info(flash, sec_addr, sector, true);
                }
                // iterator is interrupted when callback return true
                if callback(sector) {
                    return;
                }
            }
            sec_addr = self.get_next_sector_addr(sector, traversed_len);
            if sec_addr == FDB_FAILED_ADDR {
                break;
            }
        }
    }

    /// c: fdb_kvdb.c:915-938 — alloc_kv
    ///
    /// Allocate space for a KV. Returns the empty_kv address or `FDB_FAILED_ADDR`.
    /// `sector` is filled with the sector where space was found (matching C).
    pub(crate) fn alloc_kv<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        sector: &mut KvdbSecInfo,
        kv_size: usize,
    ) -> u32 {
        let mut empty_kv = FDB_FAILED_ADDR;
        let mut empty_sector = 0usize;
        let mut using_sector = 0usize;

        // c: sector_statistics_cb — count empty/using sectors
        self.sector_iterator(
            flash,
            sector,
            FdbSectorStoreStatus::Unused,
            |sec| {
                if sec.check_ok && sec.store == FdbSectorStoreStatus::Empty {
                    empty_sector += 1;
                } else if sec.check_ok && sec.store == FdbSectorStoreStatus::Using {
                    using_sector += 1;
                }
                false
            },
            false,
        );

        if using_sector > 0 {
            // alloc the KV from the using status sector first
            let gc_request = self.gc_request;
            self.sector_iterator(
                flash,
                sector,
                FdbSectorStoreStatus::Using,
                |sec| {
                    // c: alloc_kv_cb condition
                    if sec.check_ok
                        && sec.remain > kv_size + FDB_SEC_REMAIN_THRESHOLD as usize
                        && (sec.dirty == FdbSectorDirtyStatus::False
                            || (sec.dirty == FdbSectorDirtyStatus::True && !gc_request))
                    {
                        empty_kv = sec.empty_kv;
                        return true;
                    }
                    false
                },
                true,
            );
        }
        if empty_sector > 0 && empty_kv == FDB_FAILED_ADDR {
            if empty_sector > FDB_GC_EMPTY_SEC_THRESHOLD as usize || self.gc_request {
                let gc_request = self.gc_request;
                self.sector_iterator(
                    flash,
                    sector,
                    FdbSectorStoreStatus::Empty,
                    |sec| {
                        if sec.check_ok
                            && sec.remain > kv_size + FDB_SEC_REMAIN_THRESHOLD as usize
                            && (sec.dirty == FdbSectorDirtyStatus::False
                                || (sec.dirty == FdbSectorDirtyStatus::True && !gc_request))
                        {
                            empty_kv = sec.empty_kv;
                            return true;
                        }
                        false
                    },
                    true,
                );
            } else {
                // no space for new KV now will GC and retry
                self.gc_request = true;
            }
        }

        empty_kv
    }

    /// c: fdb_kvdb.c:940-1001 — del_kv
    ///
    /// Delete a KV. `old_kv` is used directly when provided; otherwise the KV is
    /// found by `key`. `complete_del=false` → PRE_DELETE; `true` → DELETED.
    ///
    /// `name` / `name_len` are only consumed by the `kv_cache` branch below.
    #[cfg_attr(not(feature = "kv_cache"), allow(unused_variables))]
    pub(crate) fn del_kv<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        key: Option<&[u8]>,
        old_kv: Option<&FdbKv>,
        complete_del: bool,
    ) -> Result<(), FdbErr> {
        let mut local_kv = FdbKv::default();
        // need find KV when old_kv is None
        let (addr_start, name, name_len) = if let Some(okv) = old_kv {
            (okv.addr_start, okv.name, okv.name_len)
        } else {
            let key = key.ok_or(FdbErr::KvNameErr)?;
            if !self.find_kv(flash, key, &mut local_kv) {
                return Err(FdbErr::KvNameErr);
            }
            (local_kv.addr_start, local_kv.name, local_kv.name_len)
        };

        let mut status_table = [0u8; KV_STATUS_TABLE_SIZE];
        // change and save the new status
        if !complete_del {
            write_status(
                flash,
                addr_start,
                &mut status_table,
                FDB_KV_STATUS_NUM as usize,
                FdbKvStatus::PreDelete as usize,
            )?;
            self.last_is_complete_del = true;
        } else {
            write_status(
                flash,
                addr_start,
                &mut status_table,
                FDB_KV_STATUS_NUM as usize,
                FdbKvStatus::Deleted as usize,
            )?;

            if !self.last_is_complete_del {
                #[cfg(feature = "kv_cache")]
                {
                    // delete the KV in flash and cache
                    if let Some(key) = key {
                        self.update_kv_cache(key, FDB_DATA_UNUSED);
                    } else {
                        self.update_kv_cache(&name[..name_len as usize], FDB_DATA_UNUSED);
                    }
                }
            }
            self.last_is_complete_del = false;
        }

        // c: read and change the sector dirty status
        let dirty_status_addr =
            align_down(addr_start, self.db_sec_size()) + SECTOR_DIRTY_OFFSET as u32;
        let mut dirty_table = [0u8; FDB_DIRTY_STATUS_TABLE_SIZE];
        if read_status(
            flash,
            dirty_status_addr,
            &mut dirty_table,
            FDB_SECTOR_DIRTY_STATUS_NUM as usize,
        ) == FdbSectorDirtyStatus::False as usize
        {
            write_status(
                flash,
                dirty_status_addr,
                &mut dirty_table,
                FDB_SECTOR_DIRTY_STATUS_NUM as usize,
                FdbSectorDirtyStatus::True as usize,
            )?;
            #[cfg(feature = "kv_cache")]
            {
                let sec_addr = align_down(addr_start, self.db_sec_size());
                if let Some(mut sec_cache) = self.get_sector_from_cache(sec_addr) {
                    sec_cache.dirty = FdbSectorDirtyStatus::True;
                    self.update_sector_cache(&sec_cache);
                }
            }
        }

        Ok(())
    }

    /// c: fdb_kvdb.c:1006-1067 — move_kv
    ///
    /// Move a KV to new space (used by GC and recovery). `goto __exit` →
    /// sequential del_kv(true) at the end.
    pub(crate) fn move_kv<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        kv: &FdbKv,
    ) -> Result<(), FdbErr> {
        let mut result = Ok(());
        let mut sector = KvdbSecInfo::default();

        // prepare to delete the current KV
        if kv.status == FdbKvStatus::Write {
            result = self.del_kv(flash, None, Some(kv), false);
        }

        let kv_addr = if result.is_ok() {
            // c: alloc_kv (NOT new_kv — no GC retry inside move_kv)
            let addr = self.alloc_kv(flash, &mut sector, kv.len as usize);
            if addr == FDB_FAILED_ADDR {
                return Err(FdbErr::SavedFull);
            }
            addr
        } else {
            return result;
        };

        let mut skip_move = false;
        if self.in_recovery_check && kv.status == FdbKvStatus::PreDelete {
            // check the KV in flash is already create success
            let name = &kv.name[..kv.name_len as usize];
            let mut kv_bak = FdbKv::default();
            if self.find_kv_no_cache(flash, name, &mut kv_bak) {
                // already create success, don't need to duplicate
                result = Ok(());
                skip_move = true;
            }
        }

        if !skip_move {
            // start move the KV
            result = self.update_sec_status(flash, &mut sector, kv.len as usize, None);
            if result.is_ok() {
                let mut status_table = [0u8; KV_STATUS_TABLE_SIZE];
                // c: _fdb_write_status(..., PRE_WRITE, false)
                write_status(
                    flash,
                    kv_addr,
                    &mut status_table,
                    FDB_KV_STATUS_NUM as usize,
                    FdbKvStatus::PreWrite as usize,
                )?;
                // copy bytes from kv->addr.start + KV_MAGIC_OFFSET to kv_addr + KV_MAGIC_OFFSET
                let kv_len = kv.len - KV_MAGIC_OFFSET as u32;
                let mut buf = [0u8; 32];
                let mut len = 0u32;
                while len < kv_len {
                    let size = if len + 32 < kv_len { 32 } else { kv_len - len };
                    let read_size = wg_align(size) as usize;
                    let _ = flash_read(
                        flash,
                        kv.addr_start + KV_MAGIC_OFFSET as u32 + len,
                        &mut buf[..read_size],
                    );
                    flash_write(
                        flash,
                        kv_addr + KV_MAGIC_OFFSET as u32 + len,
                        &buf[..size as usize],
                    )?;
                    len += size;
                }
                // c: _fdb_write_status(..., WRITE, true)
                let mut status_table2 = [0u8; KV_STATUS_TABLE_SIZE];
                write_status(
                    flash,
                    kv_addr,
                    &mut status_table2,
                    FDB_KV_STATUS_NUM as usize,
                    FdbKvStatus::Write as usize,
                )?;
                #[cfg(feature = "kv_cache")]
                {
                    self.update_sector_empty_addr_cache(
                        align_down(kv_addr, self.db_sec_size()),
                        kv_addr + KV_HDR_DATA_SIZE + wg_align(kv.name_len as u32)
                            + wg_align(kv.value_len),
                    );
                    self.update_kv_cache(&kv.name[..kv.name_len as usize], kv_addr);
                }
            }
        }

        // __exit: del_kv(db, NULL, kv, true)
        let del_result = self.del_kv(flash, None, Some(kv), true);
        // c: return result (the move result, not the exit del result — but C returns result
        // after __exit which runs del_kv; if del_kv fails C still returns the earlier result).
        // Match C: result is the move result; the exit del_kv failure is not propagated in C
        // (C assigns result = del_kv(...) only inside the move block, not at __exit). Actually
        // C's __exit: del_kv(db, NULL, kv, true); return result; — the del_kv return value is
        // discarded. So we discard del_result and return the move result.
        let _ = del_result;
        result
    }

    /// c: fdb_kvdb.c:1069-1089 — new_kv (`goto __retry` → `loop { continue }`)
    pub(crate) fn new_kv<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        sector: &mut KvdbSecInfo,
        kv_size: usize,
    ) -> u32 {
        let mut already_gc = false;
        loop {
            // __retry
            let empty_kv = self.alloc_kv(flash, sector, kv_size);
            if empty_kv == FDB_FAILED_ADDR {
                if self.gc_request && !already_gc {
                    self.gc_collect_by_free_size(flash, kv_size);
                    already_gc = true;
                    continue; // goto __retry
                } else if already_gc {
                    self.gc_request = false;
                }
            }
            return empty_kv;
        }
    }

    /// c: fdb_kvdb.c:1091-1096 — new_kv_ex
    pub(crate) fn new_kv_ex<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        sector: &mut KvdbSecInfo,
        key_len: usize,
        buf_len: usize,
    ) -> u32 {
        let kv_len = KV_HDR_DATA_SIZE + wg_align(key_len as u32) + wg_align(buf_len as u32);
        self.new_kv(flash, sector, kv_len as usize)
    }

    /// c: fdb_kvdb.c:1184-1265 — create_kv_blob
    ///
    /// Create a new KV: compute CRC, write header, name, value, set status WRITE.
    pub(crate) fn create_kv_blob<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        sector: &mut KvdbSecInfo,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), FdbErr> {
        let mut is_full = false;
        let mut kv_addr = sector.empty_kv;

        if key.len() > FDB_KV_NAME_MAX {
            return Err(FdbErr::KvNameErr);
        }

        let mut kv_hdr = KvHdrData {
            magic: KV_MAGIC_WORD,
            name_len: key.len() as u8,
            value_len: value.len() as u32,
            len: KV_HDR_DATA_SIZE + wg_align(key.len() as u32) + wg_align(value.len() as u32),
            ..Default::default() // memset to FDB_BYTE_ERASED
        };

        if kv_hdr.len > self.db_sec_size() - SECTOR_HDR_DATA_SIZE {
            return Err(FdbErr::SavedFull);
        }

        if kv_addr == FDB_FAILED_ADDR {
            kv_addr = self.new_kv(flash, sector, kv_hdr.len as usize);
        }
        if kv_addr == FDB_FAILED_ADDR {
            return Err(FdbErr::SavedFull);
        }

        // update the sector status
        let mut result = self.update_sec_status(flash, sector, kv_hdr.len as usize, Some(&mut is_full));
        if result.is_ok() {
            // start calculate CRC32 (name_len[4] + value_len[4] + name + name_pad + value + value_pad)
            let hdr_buf = kv_hdr.to_bytes();
            let mut crc = 0u32;
            crc = calc_crc32(crc, &hdr_buf[KV_NAME_LEN_OFFSET..KV_NAME_LEN_OFFSET + 4]);
            let vloff = KV_NAME_LEN_OFFSET + 4;
            crc = calc_crc32(crc, &hdr_buf[vloff..vloff + 4]);
            crc = calc_crc32(crc, key);
            let name_pad = wg_align(key.len() as u32) as usize - key.len();
            for _ in 0..name_pad {
                crc = calc_crc32(crc, &[FDB_BYTE_ERASED]);
            }
            crc = calc_crc32(crc, value);
            let value_pad = wg_align(value.len() as u32) as usize - value.len();
            for _ in 0..value_pad {
                crc = calc_crc32(crc, &[FDB_BYTE_ERASED]);
            }
            kv_hdr.crc32 = crc;
            // write KV header data
            result = write_kv_hdr(flash, kv_addr, &mut kv_hdr);
        }
        // write key name
        if result.is_ok() {
            result = flash_write_align(flash, kv_addr + KV_HDR_DATA_SIZE, key);
            #[cfg(feature = "kv_cache")]
            {
                if !is_full {
                    self.update_sector_empty_addr_cache(
                        sector.addr,
                        kv_addr + KV_HDR_DATA_SIZE + wg_align(key.len() as u32)
                            + wg_align(value.len() as u32),
                    );
                }
                self.update_kv_cache(key, kv_addr);
            }
        }
        // write value
        if result.is_ok() {
            result = flash_write_align(
                flash,
                kv_addr + KV_HDR_DATA_SIZE + wg_align(key.len() as u32),
                value,
            );
        }
        // change the KV status to WRITE
        if result.is_ok() {
            let mut status_table = kv_hdr.status_table;
            result = write_status(
                flash,
                kv_addr,
                &mut status_table,
                FDB_KV_STATUS_NUM as usize,
                FdbKvStatus::Write as usize,
            );
        }
        // trigger GC collect when current sector is full
        if result.is_ok() && is_full {
            self.gc_request = true;
        }

        result
    }

    /// c: fdb_kvdb.c:1295-1327 — set_kv
    ///
    /// Set a KV (create new + delete old). `value == None` → delete.
    pub(crate) fn set_kv<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        key: &[u8],
        value: Option<&[u8]>,
    ) -> Result<(), FdbErr> {
        match value {
            None => self.del_kv(flash, Some(key), None, true),
            Some(buf) => {
                let mut cur_sector = KvdbSecInfo::default();
                let mut cur_kv = FdbKv::default();

                // make sure the flash has enough space
                if self.new_kv_ex(flash, &mut cur_sector, key.len(), buf.len()) == FDB_FAILED_ADDR {
                    return Err(FdbErr::SavedFull);
                }
                let kv_is_found = self.find_kv(flash, key, &mut cur_kv);
                // prepare to delete the old KV
                let mut result = Ok(());
                if kv_is_found {
                    result = self.del_kv(flash, Some(key), Some(&cur_kv), false);
                }
                // create the new KV
                if result.is_ok() {
                    result = self.create_kv_blob(flash, &mut cur_sector, key, buf);
                }
                // delete the old KV
                if kv_is_found && result.is_ok() {
                    result = self.del_kv(flash, Some(key), Some(&cur_kv), true);
                }
                // process the GC after set KV
                if self.gc_request {
                    self.gc_collect_by_free_size(
                        flash,
                        (KV_HDR_DATA_SIZE + wg_align(key.len() as u32) + wg_align(buf.len() as u32))
                            as usize,
                    );
                }
                result
            }
        }
    }

    // ===== GC (c: fdb_kvdb.c:1098-1181) =====
    //
    // C used `sector_iterator` with a `do_gc` callback that mutated db. Rust can't
    // hold `&mut self` across a callback that also needs `&mut self`, so the GC
    // sector walk is an explicit loop (reborrowing `&mut self` per sector). The
    // empty-sector count still uses the read-only `sector_iterator` callback.

    /// c: fdb_kvdb.c:1112-1151 — do_gc (one sector)
    ///
    /// Returns `true` to stop GC (enough free space collected).
    fn do_gc<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        sector: &mut KvdbSecInfo,
        setting_free_size: usize,
        last_gc_sec_addr: &mut u32,
    ) -> bool {
        if sector.check_ok
            && (sector.dirty == FdbSectorDirtyStatus::True
                || sector.dirty == FdbSectorDirtyStatus::Gc)
        {
            let mut status_table = [0u8; FDB_DIRTY_STATUS_TABLE_SIZE];
            // change the sector status to GC
            let _ = write_status(
                flash,
                sector.addr + SECTOR_DIRTY_OFFSET as u32,
                &mut status_table,
                FDB_SECTOR_DIRTY_STATUS_NUM as usize,
                FdbSectorDirtyStatus::Gc as usize,
            );
            // search all KV
            let mut kv = FdbKv {
                addr_start: sector.addr + SECTOR_HDR_DATA_SIZE,
                ..Default::default()
            };
            loop {
                let _ = self.read_kv(flash, &mut kv);
                if kv.crc_is_ok
                    && (kv.status == FdbKvStatus::Write || kv.status == FdbKvStatus::PreDelete)
                {
                    // move the KV to new space
                    let _ = self.move_kv(flash, &kv);
                }
                let next = self.get_next_kv_addr(flash, sector, &kv);
                if next == FDB_FAILED_ADDR {
                    break;
                }
                kv.addr_start = next;
            }
            let _ = self.format_sector(flash, sector.addr, SECTOR_NOT_COMBINED);
            let prev_last = *last_gc_sec_addr;
            *last_gc_sec_addr = sector.addr;
            // update oldest_addr for next GC sector format
            self.parent.oldest_addr = self.get_next_sector_addr(sector, 0);
            // the collect new space is in last GC sector
            let mut last_gc_sector = KvdbSecInfo::default();
            if self
                .read_sector_info(flash, prev_last, &mut last_gc_sector, true)
                .is_ok()
                && last_gc_sector.remain > setting_free_size
            {
                return true;
            }
        }
        false
    }

    /// c: fdb_kvdb.c:1153-1171 — gc_collect_by_free_size
    pub(crate) fn gc_collect_by_free_size<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        free_size: usize,
    ) {
        let mut sector = KvdbSecInfo::default();
        let mut empty_sec_num = 0usize;
        let mut empty_sec_addr = 0u32;

        // c: gc_check_cb — count empty sectors (read-only callback)
        self.sector_iterator(
            flash,
            &mut sector,
            FdbSectorStoreStatus::Empty,
            |sec| {
                if sec.check_ok {
                    empty_sec_num += 1;
                    empty_sec_addr = sec.addr;
                }
                false
            },
            false,
        );

        // do GC collect
        if empty_sec_num <= FDB_GC_EMPTY_SEC_THRESHOLD as usize {
            // c: do_gc needs &mut self + &mut flash → explicit loop (not callback iterator)
            let mut last_gc_sec_addr = empty_sec_addr;
            let mut sec_addr = self.db_oldest_addr();
            let mut traversed_len = 0u32;
            loop {
                traversed_len += self.db_sec_size();
                if self
                    .read_sector_info(flash, sec_addr, &mut sector, false)
                    .is_ok()
                {
                    let stop = self.do_gc(flash, &mut sector, free_size, &mut last_gc_sec_addr);
                    if stop {
                        break;
                    }
                }
                sec_addr = self.get_next_sector_addr(&sector, traversed_len);
                if sec_addr == FDB_FAILED_ADDR {
                    break;
                }
            }
        }

        self.gc_request = false;
    }

    /// c: fdb_kvdb.c:1178-1181 — gc_collect
    pub(crate) fn gc_collect<F: FlashDevice>(&mut self, flash: &mut F) {
        self.gc_collect_by_free_size(flash, self.db_max_size() as usize);
    }

    // ===== Public CRUD API (c: fdb_kvdb.c:655-753, 1275-1378) =====

    /// c: fdb_kvdb.c:655-673 — fdb_kv_get_obj
    ///
    /// Get a KV object by key. Returns `true` if found; `kv` is filled.
    pub fn kv_get_obj<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        key: &str,
        kv: &mut FdbKv,
    ) -> bool {
        if !self.db_init_ok() {
            return false;
        }
        self.db_lock();
        let found = self.find_kv(flash, key.as_bytes(), kv);
        self.db_unlock();
        found
    }

    /// c: fdb_kvdb.c:701-719 — fdb_kv_get_blob
    ///
    /// Get a blob KV value. Returns `Some(read_len)` (0 when not found).
    pub fn kv_get_blob<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        key: &str,
        blob: &mut FdbBlob,
    ) -> Option<usize> {
        if !self.db_init_ok() {
            return None;
        }
        self.db_lock();
        let read_len = self.get_kv(flash, key.as_bytes(), blob.buf, Some(&mut blob.saved_len));
        self.db_unlock();
        Some(read_len)
    }

    /// c: fdb_kvdb.c:732-753 — fdb_kv_get
    ///
    /// Get a string KV value. Returns `None` when not found / not a string / not init.
    pub fn kv_get<F: FlashDevice>(&mut self, flash: &mut F, key: &str) -> Option<String> {
        if !self.db_init_ok() {
            return None;
        }
        self.db_lock();
        let mut buf = [0u8; FDB_STR_KV_VALUE_MAX_SIZE];
        let mut blob = blob_make(&mut buf);
        let get_size = self.get_kv(
            flash,
            key.as_bytes(),
            blob.buf,
            Some(&mut blob.saved_len),
        );
        self.db_unlock();

        if get_size > 0 && fdb_is_str(&buf[..get_size]) {
            // fdb_is_str guarantees printable ASCII (valid UTF-8)
            String::from_utf8(buf[..get_size].to_vec()).ok()
        } else {
            None
        }
    }

    /// c: fdb_kvdb.c:1275-1293 — fdb_kv_del
    pub fn kv_del<F: FlashDevice>(&mut self, flash: &mut F, key: &str) -> Result<(), FdbErr> {
        if !self.db_init_ok() {
            return Err(FdbErr::InitFailed);
        }
        self.db_lock();
        let result = self.del_kv(flash, Some(key.as_bytes()), None, true);
        self.db_unlock();
        result
    }

    /// c: fdb_kvdb.c:1339-1357 — fdb_kv_set_blob
    pub fn kv_set_blob<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        key: &str,
        blob: &mut FdbBlob,
    ) -> Result<(), FdbErr> {
        if !self.db_init_ok() {
            return Err(FdbErr::InitFailed);
        }
        self.db_lock();
        let value: &[u8] = blob.buf;
        let result = self.set_kv(flash, key.as_bytes(), Some(value));
        self.db_unlock();
        result
    }

    /// c: fdb_kvdb.c:1369-1378 — fdb_kv_set
    ///
    /// Set a string KV. (C's NULL-value delete path is exposed via `kv_del`.)
    pub fn kv_set<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        key: &str,
        value: &str,
    ) -> Result<(), FdbErr> {
        if !self.db_init_ok() {
            return Err(FdbErr::InitFailed);
        }
        self.db_lock();
        let result = self.set_kv(flash, key.as_bytes(), Some(value.as_bytes()));
        self.db_unlock();
        result
    }
}

/// c: fdb_kvdb.c:683-690 — fdb_kv_to_blob
///
/// Convert a KV object to a blob object (free function, no db needed).
pub fn kv_to_blob(kv: &FdbKv, blob: &mut FdbBlob) {
    blob.saved_meta_addr = kv.addr_start;
    blob.saved_addr = kv.addr_value;
    blob.saved_len = kv.value_len as usize;
}

/// c: fdb_def.h compatibility — strlen over a byte slice (for default KV value_len==0).
fn strlen_bytes(bytes: &[u8]) -> usize {
    bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())
}

// ===== Recovery / init / iterator / check / print / set_default (c: fdb_kvdb.c:1386-1942) =====

impl FdbKvdb {
    /// c: fdb_kvdb.c:1563-1579 — check_sec_hdr_cb (restructured: collect then format)
    ///
    /// Iterate all sectors; count those with invalid headers. When `not_formatable`
    /// is false the failed sectors are formatted. Returns the failed count.
    fn check_sec_hdr<F: FlashDevice>(&mut self, flash: &mut F) -> usize {
        let mut failed_count = 0usize;
        let mut failed_addrs: Vec<u32> = Vec::new();
        let mut sector = KvdbSecInfo::default();
        let mut sec_addr = self.db_oldest_addr();
        let mut traversed_len = 0u32;
        loop {
            traversed_len += self.db_sec_size();
            let _ = self.read_sector_info(flash, sec_addr, &mut sector, false);
            if !sector.check_ok {
                failed_count += 1;
                if self.parent.not_formatable {
                    // c: return true (stop iteration) — not_formatable stops on first failure
                    return failed_count;
                } else {
                    failed_addrs.push(sec_addr);
                }
            }
            sec_addr = self.get_next_sector_addr(&sector, traversed_len);
            if sec_addr == FDB_FAILED_ADDR {
                break;
            }
        }
        // format the failed sectors (c: format_sector inside the callback)
        for addr in failed_addrs {
            let _ = self.format_sector(flash, addr, SECTOR_NOT_COMBINED);
        }
        failed_count
    }

    /// c: fdb_kvdb.c:1581-1593 — check_and_recovery_gc_cb (restructured)
    ///
    /// Resume any interrupted GC (sectors with dirty == GC). Sets `gc_request`
    /// and runs `gc_collect` once when any GC-dirty sector is found (the extra
    /// per-sector calls in C are no-ops since the first `gc_collect` formats them).
    fn check_and_recovery_gc<F: FlashDevice>(&mut self, flash: &mut F) {
        let mut sector = KvdbSecInfo::default();
        let mut sec_addr = self.db_oldest_addr();
        let mut traversed_len = 0u32;
        let mut need_gc = false;
        loop {
            traversed_len += self.db_sec_size();
            let _ = self.read_sector_info(flash, sec_addr, &mut sector, false);
            if sector.check_ok && sector.dirty == FdbSectorDirtyStatus::Gc {
                need_gc = true;
            }
            sec_addr = self.get_next_sector_addr(&sector, traversed_len);
            if sec_addr == FDB_FAILED_ADDR {
                break;
            }
        }
        if need_gc {
            // c: db->gc_request = true; gc_collect(db);
            self.gc_request = true;
            self.gc_collect(flash);
        }
    }

    /// c: fdb_kvdb.c:1595-1623 — check_and_recovery_kv_cb (restructured: collect then process)
    ///
    /// Iterate all KVs collecting recovery actions, stopping at the first PRE_WRITE
    /// (matching C's `return true`). Then process: PRE_WRITE → ERR_HDR; PRE_DELETE →
    /// move_kv; WRITE → cache update (kv_cache feature only).
    fn check_and_recovery_kv<F: FlashDevice>(&mut self, flash: &mut F) {
        let mut pre_delete_kvs: Vec<FdbKv> = Vec::new();
        #[cfg(feature = "kv_cache")]
        let mut write_kvs: Vec<FdbKv> = Vec::new();
        let mut pre_write_addr: Option<u32> = None;

        let mut kv = FdbKv::default();
        // c: kv_iterator(db, &kv, db, NULL, check_and_recovery_kv_cb)
        self.kv_iterator(flash, &mut kv, |_flash, cur_kv| {
            if cur_kv.crc_is_ok && cur_kv.status == FdbKvStatus::PreDelete {
                // c: recovery the prepare deleted KV (move_kv happens after iteration)
                pre_delete_kvs.push(cur_kv.clone());
                false // continue iteration
            } else if cur_kv.status == FdbKvStatus::PreWrite {
                // c: change the status to error, return true (stop)
                pre_write_addr = Some(cur_kv.addr_start);
                true // stop iteration
            } else {
                // c: FDB_KV_WRITE branch — update cache (when enabled) then continue.
                // The return value is always `false` (continue iteration), matching C's
                // trailing `return false` after the cache side effect.
                #[cfg(feature = "kv_cache")]
                {
                    if cur_kv.crc_is_ok && cur_kv.status == FdbKvStatus::Write {
                        write_kvs.push(cur_kv.clone());
                    }
                }
                false
            }
        });

        // process PRE_WRITE: change the status to error
        if let Some(addr) = pre_write_addr {
            let mut status_table = [0u8; KV_STATUS_TABLE_SIZE];
            let _ = write_status(
                flash,
                addr,
                &mut status_table,
                FDB_KV_STATUS_NUM as usize,
                FdbKvStatus::ErrHdr as usize,
            );
        }
        // process PRE_DELETE: move each KV to new space
        for pkv in &pre_delete_kvs {
            let _ = self.move_kv(flash, pkv);
        }
        // process WRITE: update cache
        #[cfg(feature = "kv_cache")]
        for wkv in &write_kvs {
            self.update_kv_cache(&wkv.name[..wkv.name_len as usize], wkv.addr_start);
        }
    }

    /// c: fdb_kvdb.c:1630-1663 — _fdb_kv_load (`goto __retry` → `loop { continue }`)
    fn fdb_kv_load<F: FlashDevice>(&mut self, flash: &mut F) -> Result<(), FdbErr> {
        self.in_recovery_check = true;

        // check all sector header
        let check_failed_count = self.check_sec_hdr(flash);
        if self.parent.not_formatable && check_failed_count > 0 {
            return Err(FdbErr::ReadErr);
        }
        // all sector header check failed → set to default
        if check_failed_count == self.sector_num() as usize {
            self.kv_set_default(flash)?;
        }

        // check all sector header for recovery GC
        self.check_and_recovery_gc(flash);

        // __retry: check all KV for recovery
        loop {
            self.check_and_recovery_kv(flash);
            if self.gc_request {
                self.gc_collect(flash);
                continue; // goto __retry
            }
            break;
        }

        self.in_recovery_check = false;
        Ok(())
    }

    /// c: fdb_kvdb.c:1386-1431 — fdb_kv_set_default
    ///
    /// Recovery all KV to default: format all sectors, then create each default KV.
    pub fn kv_set_default<F: FlashDevice>(&mut self, flash: &mut F) -> Result<(), FdbErr> {
        self.db_lock();

        #[cfg(feature = "kv_cache")]
        {
            use crate::def::FDB_KV_CACHE_TABLE_SIZE;
            for i in 0..FDB_KV_CACHE_TABLE_SIZE {
                self.kv_cache_table[i].addr = FDB_DATA_UNUSED;
            }
        }

        let mut result = Ok(());
        // format all sectors
        let mut addr = 0u32;
        while addr < self.db_max_size() {
            result = self.format_sector(flash, addr, SECTOR_NOT_COMBINED);
            if result.is_err() {
                break; // goto __exit
            }
            addr += self.db_sec_size();
        }
        // create default KV
        if result.is_ok() {
            for node in self.default_kvs.kvs.iter() {
                // c: value_len==0 → treat as string (strlen), for V4.0 compatibility
                let value_len = if node.value_len == 0 {
                    strlen_bytes(node.value)
                } else {
                    node.value_len
                };
                let mut sector = KvdbSecInfo {
                    empty_kv: FDB_FAILED_ADDR,
                    ..Default::default()
                };
                // c: create_kv_blob return is NOT assigned to result (C source behaviour)
                let _ = self.create_kv_blob(flash, &mut sector, node.key.as_bytes(), &node.value[..value_len]);
                // c: if (result != OK) goto __exit — checks the format result, stays OK here
            }
        }

        // __exit:
        self.parent.oldest_addr = 0;
        self.db_unlock();
        result
    }

    /// c: fdb_kvdb.c:1432-1507 — fdb_kv_print
    ///
    /// Print all KV. C used FDB_PRINT; this returns the formatted output as a String
    /// (the `goto __reload` forward jump is restructured: the value is collected
    /// during the is_str check and printed directly).
    pub fn kv_print<F: FlashDevice>(&mut self, flash: &mut F) -> String {
        if !self.db_init_ok() {
            return String::new();
        }
        self.db_lock();

        let mut output = String::new();
        let mut using_size: usize = 0;
        let mut kv = FdbKv::default();
        // c: kv_iterator(db, &kv, &using_size, db, print_kv_cb)
        self.kv_iterator(flash, &mut kv, |fl, cur_kv| {
            if cur_kv.crc_is_ok {
                // calculate the total using flash size
                using_size += cur_kv.len as usize;
                if cur_kv.status == FdbKvStatus::Write {
                    // c: FDB_PRINT("%.*s=", name)
                    output.push_str(cur_kv.name_str());
                    output.push('=');
                    if (cur_kv.value_len as usize) < FDB_STR_KV_VALUE_MAX_SIZE {
                        // check the value is string (c: goto __reload re-read replaced by collect)
                        let mut buf = [0u8; 32];
                        let mut full_value: Vec<u8> = Vec::new();
                        let mut value_is_str = true;
                        let mut len = 0u32;
                        while len < cur_kv.value_len {
                            let size = if len + 32 < cur_kv.value_len {
                                32
                            } else {
                                cur_kv.value_len - len
                            };
                            let read_size = wg_align(size) as usize;
                            let _ = flash_read(fl, cur_kv.addr_value + len, &mut buf[..read_size]);
                            if fdb_is_str(&buf[..size as usize]) {
                                full_value.extend_from_slice(&buf[..size as usize]);
                            } else {
                                value_is_str = false;
                                break;
                            }
                            len += size;
                        }
                        if value_is_str {
                            output.push_str(&String::from_utf8_lossy(&full_value));
                        } else {
                            // c: FDB_PRINT("blob @0x%08X %ubytes", ...)
                            output.push_str(&format!(
                                "blob @0x{:08X} {}bytes",
                                cur_kv.addr_value, cur_kv.value_len
                            ));
                        }
                    } else {
                        // value too large to be a string
                        output.push_str(&format!(
                            "blob @0x{:08X} {}bytes",
                            cur_kv.addr_value, cur_kv.value_len
                        ));
                    }
                    output.push('\n');
                }
            }
            false // keep iterating
        });

        // c: summary
        let sector_num = self.sector_num();
        output.push_str("\nmode: next generation\n");
        output.push_str(&format!(
            "size: {}/{} bytes.\n",
            using_size + ((sector_num - FDB_GC_EMPTY_SEC_THRESHOLD) * SECTOR_HDR_DATA_SIZE) as usize,
            self.db_max_size() as usize
                - self.db_sec_size() as usize * FDB_GC_EMPTY_SEC_THRESHOLD as usize
        ));

        self.db_unlock();
        output
    }

    /// c: fdb_kvdb.c:1513-1543 — kv_auto_update (feature-gated)
    ///
    /// Auto update KV to latest default when the saved version number differs.
    /// C stored `db->ver_num` as `size_t`; the Rust `FdbKvdb::ver_num` is `u32`,
    /// so the on-flash format is a 4-byte native-endian u32.
    #[cfg(feature = "kv_auto_update")]
    fn kv_auto_update<F: FlashDevice>(&mut self, flash: &mut F) {
        let setting_ver_num = self.ver_num;
        let mut saved_ver_len = 0usize;
        let mut buf = [0u8; 4];
        let read_len = self.get_kv(
            flash,
            VER_NUM_KV_NAME.as_bytes(),
            &mut buf,
            Some(&mut saved_ver_len),
        );
        if read_len > 0 {
            // check version number
            let saved = u32::from_ne_bytes(buf);
            if saved != setting_ver_num {
                // add a new KV when it's not found
                for node in self.default_kvs.kvs.iter() {
                    let mut cur_kv = FdbKv::default();
                    if !self.find_kv(flash, node.key.as_bytes(), &mut cur_kv) {
                        let value_len = if node.value_len == 0 {
                            strlen_bytes(node.value)
                        } else {
                            node.value_len
                        };
                        let mut sector = KvdbSecInfo::default();
                        sector.empty_kv = FDB_FAILED_ADDR;
                        let _ = self.create_kv_blob(
                            flash,
                            &mut sector,
                            node.key.as_bytes(),
                            &node.value[..value_len],
                        );
                    }
                }
            } else {
                // version number not changed now return
                return;
            }
        }
        let bytes = setting_ver_num.to_ne_bytes();
        let _ = self.set_kv(flash, VER_NUM_KV_NAME.as_bytes(), Some(&bytes));
    }

    // ===== Control (c: fdb_kvdb.c:1672-1727) → builder/setter methods =====

    /// c: FDB_KVDB_CTRL_SET_SEC_SIZE — must be called before init.
    pub fn set_sec_size(&mut self, sec_size: u32) {
        assert!(!self.parent.init_ok, "set_sec_size must be called before init");
        self.parent.sec_size = sec_size;
    }
    /// c: FDB_KVDB_CTRL_GET_SEC_SIZE
    pub fn get_sec_size(&self) -> u32 {
        self.parent.sec_size
    }
    /// c: FDB_KVDB_CTRL_SET_LOCK — install a lock callback (replaces C's void* fn-ptr cast).
    pub fn set_lock(&mut self, lock: fn(&mut FdbDb)) {
        self.parent.lock = Some(lock);
    }
    /// c: FDB_KVDB_CTRL_SET_UNLOCK — install an unlock callback.
    pub fn set_unlock(&mut self, unlock: fn(&mut FdbDb)) {
        self.parent.unlock = Some(unlock);
    }
    /// c: FDB_KVDB_CTRL_SET_FILE_MODE (only with the `file_mode` feature)
    #[cfg(feature = "file_mode")]
    pub fn set_file_mode(&mut self, file_mode: bool) {
        assert!(!self.parent.init_ok, "set_file_mode must be called before init");
        self.parent.file_mode = file_mode;
    }
    /// c: FDB_KVDB_CTRL_SET_MAX_SIZE (only with the `file_mode` feature)
    #[cfg(feature = "file_mode")]
    pub fn set_max_size(&mut self, max_size: u32) {
        assert!(!self.parent.init_ok, "set_max_size must be called before init");
        self.parent.max_size = max_size;
    }
    /// c: FDB_KVDB_CTRL_SET_NOT_FORMAT — must be called before init.
    pub fn set_not_format(&mut self, not_formatable: bool) {
        assert!(!self.parent.init_ok, "set_not_format must be called before init");
        self.parent.not_formatable = not_formatable;
    }

    /// c: fdb_kvdb.c:1740-1814 — fdb_kvdb_init
    ///
    /// Initialise the KV database: validate config, find the oldest sector, load
    /// (recovery), and optionally auto-update. The flash backend is supplied via
    /// the `FlashDevice` trait (C's FAL/file lookup is replaced by direct config).
    pub fn kvdb_init<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        name: &'static str,
        path: &'static str,
        default_kvs: FdbDefaultKv,
    ) -> Result<(), FdbErr> {
        // c: FDB_ASSERT((FDB_STR_KV_VALUE_MAX_SIZE * 8) % FDB_WRITE_GRAN == 0)
        // (holds for all valid GRAN: 1024 % {1,8,32,64,128,256} == 0)

        let init_result = init_ex(&mut self.parent, name, path, FdbDbType::Kv);
        if init_result.is_err() {
            init_finish(&mut self.parent, init_result);
            return init_result;
        }

        self.db_lock();
        self.gc_request = false;
        self.in_recovery_check = false;
        self.default_kvs = default_kvs;

        // find the oldest sector address (c: check_oldest_addr_cb)
        {
            let mut sector_oldest_addr = 0u32;
            let mut last_sector_status = FdbSectorStoreStatus::Unused;
            self.parent.oldest_addr = 0;
            let mut sector = KvdbSecInfo::default();
            self.sector_iterator(
                flash,
                &mut sector,
                FdbSectorStoreStatus::Unused,
                |sec| {
                    // c: check_oldest_addr_cb — oldest is where status goes Empty→Full/Using
                    if last_sector_status == FdbSectorStoreStatus::Empty
                        && (sec.store == FdbSectorStoreStatus::Full
                            || sec.store == FdbSectorStoreStatus::Using)
                    {
                        sector_oldest_addr = sec.addr;
                    }
                    last_sector_status = sec.store;
                    false
                },
                false,
            );
            self.parent.oldest_addr = sector_oldest_addr;
        }

        // c: FDB_ASSERT(FDB_GC_EMPTY_SEC_THRESHOLD > 0 && < SECTOR_NUM)
        assert!(
            FDB_GC_EMPTY_SEC_THRESHOLD > 0 && FDB_GC_EMPTY_SEC_THRESHOLD < self.sector_num(),
            "GC empty sector threshold must be > 0 and < sector count"
        );

        #[cfg(feature = "kv_cache")]
        {
            use crate::def::{FDB_KV_CACHE_TABLE_SIZE, FDB_SECTOR_CACHE_TABLE_SIZE};
            for i in 0..FDB_SECTOR_CACHE_TABLE_SIZE {
                self.sector_cache_table[i].check_ok = false;
                self.sector_cache_table[i].empty_kv = FDB_FAILED_ADDR;
                self.sector_cache_table[i].addr = FDB_DATA_UNUSED;
            }
            for i in 0..FDB_KV_CACHE_TABLE_SIZE {
                self.kv_cache_table[i].addr = FDB_DATA_UNUSED;
            }
        }

        self.db_unlock();

        let result = self.fdb_kv_load(flash);

        self.db_lock();
        #[cfg(feature = "kv_auto_update")]
        {
            if result.is_ok() {
                self.kv_auto_update(flash);
            }
        }
        self.db_unlock();

        init_finish(&mut self.parent, result);
        result
    }

    /// c: fdb_kvdb.c:1823-1828 — fdb_kvdb_deinit
    pub fn kvdb_deinit(&mut self) -> Result<(), FdbErr> {
        deinit(&mut self.parent);
        Ok(())
    }

    /// c: fdb_kvdb.c:1838-1850 — fdb_kv_iterator_init
    pub fn kv_iterator_init(&self) -> FdbKvIterator {
        let mut itr = FdbKvIterator::default();
        itr.curr_kv.addr_start = 0;
        itr.iterated_cnt = 0;
        itr.iterated_obj_bytes = 0;
        itr.iterated_value_bytes = 0;
        itr.traversed_len = 0;
        // c: Start from sector head
        itr.sector_addr = self.db_oldest_addr();
        itr
    }

    /// c: fdb_kvdb.c:1860-1896 — fdb_kv_iterate
    ///
    /// Advance the iterator to the next valid WRITE KV. Returns `true` when a valid
    /// KV was found (stats updated), `false` when iteration is complete.
    pub fn kv_iterate<F: FlashDevice>(
        &mut self,
        flash: &mut F,
        itr: &mut FdbKvIterator,
    ) -> bool {
        let mut sector = KvdbSecInfo::default();
        loop {
            let sec_ok = self
                .read_sector_info(flash, itr.sector_addr, &mut sector, false)
                .is_ok();
            let mut try_sector = sec_ok
                && (sector.store == FdbSectorStoreStatus::Using
                    || sector.store == FdbSectorStoreStatus::Full);
            if try_sector {
                if itr.curr_kv.addr_start == 0 {
                    // the first KV address
                    itr.curr_kv.addr_start = sector.addr + SECTOR_HDR_DATA_SIZE;
                } else {
                    let next = self.get_next_kv_addr(flash, &sector, &itr.curr_kv);
                    if next == FDB_FAILED_ADDR {
                        // exhausted this sector's KVs → move to next sector
                        try_sector = false;
                    } else {
                        itr.curr_kv.addr_start = next;
                    }
                }
            }
            if try_sector {
                // inner loop over KVs in this sector
                loop {
                    let _ = self.read_kv(flash, &mut itr.curr_kv);
                    if itr.curr_kv.status == FdbKvStatus::Write && itr.curr_kv.crc_is_ok {
                        // got a valid KV — update stats and return
                        itr.iterated_cnt += 1;
                        itr.iterated_obj_bytes += itr.curr_kv.len as usize;
                        itr.iterated_value_bytes += itr.curr_kv.value_len as usize;
                        return true;
                    }
                    let next = self.get_next_kv_addr(flash, &sector, &itr.curr_kv);
                    if next == FDB_FAILED_ADDR {
                        break;
                    }
                    itr.curr_kv.addr_start = next;
                }
            }
            // move to next sector
            itr.curr_kv.addr_start = 0;
            itr.traversed_len += self.db_sec_size();
            itr.sector_addr = self.get_next_sector_addr(&sector, itr.traversed_len);
            if itr.sector_addr == FDB_FAILED_ADDR {
                return false;
            }
        }
    }

    /// c: fdb_kvdb.c:1905-1942 — fdb_kvdb_check
    ///
    /// Integrity check: read every sector and KV, returning the first error.
    pub fn kvdb_check<F: FlashDevice>(&mut self, flash: &mut F) -> Result<(), FdbErr> {
        if !self.db_init_ok() {
            return Err(FdbErr::InitFailed);
        }
        self.db_lock();

        let mut sec_addr = self.db_oldest_addr();
        let mut traversed_len = 0u32;
        let mut sector = KvdbSecInfo::default();
        let mut kv = FdbKv::default();
        // `result` is assigned at the top of every loop iteration before any
        // `break`, so it is always initialized when read below.
        let mut result: Result<(), FdbErr>;
        loop {
            traversed_len += self.db_sec_size();
            result = self.read_sector_info(flash, sec_addr, &mut sector, false);
            if result.is_ok()
                && (sector.store == FdbSectorStoreStatus::Using
                    || sector.store == FdbSectorStoreStatus::Full)
            {
                kv.addr_start = sector.addr + SECTOR_HDR_DATA_SIZE;
                loop {
                    result = self.read_kv(flash, &mut kv);
                    if result.is_err() {
                        break;
                    }
                    let next = self.get_next_kv_addr(flash, &sector, &kv);
                    if next == FDB_FAILED_ADDR {
                        break;
                    }
                    kv.addr_start = next;
                }
            }
            if result.is_err() {
                break;
            }
            sec_addr = self.get_next_sector_addr(&sector, traversed_len);
            if sec_addr == FDB_FAILED_ADDR {
                break;
            }
        }

        self.db_unlock();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::def::FdbDefaultKvNode;
    use crate::low_lvl::blob_read;
    // ---- on-flash struct sizes (GRAN==1, the default configuration) ----

    #[test]
    fn test_sector_hdr_size() {
        // c: sizeof(struct sector_hdr_data) for GRAN==1
        // status_table(1+1) + 2 align pad + magic(4) + combined(4) + reserved(4) = 16
        assert_eq!(core::mem::size_of::<SectorHdrData>(), 16);
        assert_eq!(SECTOR_HDR_DATA_SIZE, 16);
    }

    #[test]
    fn test_kv_hdr_size() {
        // c: sizeof(struct kv_hdr_data) for GRAN==1
        // status(1) + 3 align pad + magic(4) + len(4) + crc32(4) + name_len(1)
        // + 3 align pad + value_len(4) = 24
        assert_eq!(core::mem::size_of::<KvHdrData>(), 24);
        assert_eq!(KV_HDR_DATA_SIZE, 24);
    }

    #[test]
    fn test_header_layouts() {
        // Combined layout check for the default GRAN==1 configuration.
        assert_eq!(core::mem::size_of::<SectorHdrData>(), 16);
        assert_eq!(core::mem::size_of::<KvHdrData>(), 24);
        assert_eq!(SECTOR_HDR_DATA_SIZE, 16);
        assert_eq!(KV_HDR_DATA_SIZE, 24);
    }

    #[test]
    fn test_offsets() {
        // c: fdb_kvdb.c:76-82 — offsets must match C offsetof for GRAN==1
        // SectorHdrData: store@0, dirty@1, magic@4 (2 bytes align pad after dirty)
        assert_eq!(SECTOR_STORE_OFFSET, 0);
        assert_eq!(SECTOR_DIRTY_OFFSET, FDB_STORE_STATUS_TABLE_SIZE);
        assert_eq!(SECTOR_MAGIC_OFFSET, 4);
        // KvHdrData: status@0, magic@4 (3 bytes align pad), len@8, name_len@16
        assert_eq!(KV_MAGIC_OFFSET, 4);
        assert_eq!(KV_LEN_OFFSET, 8);
        assert_eq!(KV_NAME_LEN_OFFSET, 16);
        // KV_STATUS_TABLE_SIZE for GRAN==1 == (6*1+7)/8 == 1
        assert_eq!(KV_STATUS_TABLE_SIZE, 1);
    }

    #[test]
    fn test_status_table_sizes() {
        // c: fdb_low_lvl.h:43-44 — FDB_STORE/DIRTY_STATUS_TABLE_SIZE for GRAN==1
        assert_eq!(FDB_STORE_STATUS_TABLE_SIZE, 1);
        assert_eq!(FDB_DIRTY_STATUS_TABLE_SIZE, 1);
    }

    #[test]
    fn test_sector_hdr_round_trip() {
        // Build a sector header, serialise, deserialise, and verify field fidelity.
        let mut hdr = SectorHdrData::default();
        // encode store status = Empty (index 1) and dirty = False (index 1)
        set_status(
            &mut hdr.status_table.store,
            FDB_SECTOR_STORE_STATUS_NUM as usize,
            FdbSectorStoreStatus::Empty as usize,
        );
        set_status(
            &mut hdr.status_table.dirty,
            FDB_SECTOR_DIRTY_STATUS_NUM as usize,
            FdbSectorDirtyStatus::False as usize,
        );
        hdr.magic = SECTOR_MAGIC_WORD;
        hdr.combined = SECTOR_NOT_COMBINED;
        hdr.reserved = FDB_DATA_UNUSED;

        let buf = hdr.to_bytes();
        // Padding bytes stay erased (0xFF) — verify the alignment-pad region.
        // For GRAN==1 the pad is at bytes [2,3) before magic@4.
        assert_eq!(buf[2], FDB_BYTE_ERASED);
        assert_eq!(buf[3], FDB_BYTE_ERASED);

        let decoded = SectorHdrData::from_bytes(&buf);
        assert_eq!(decoded.magic, SECTOR_MAGIC_WORD);
        assert_eq!(decoded.combined, SECTOR_NOT_COMBINED);
        assert_eq!(decoded.reserved, FDB_DATA_UNUSED);
        // decode the store/dirty status back
        assert_eq!(
            get_status(&decoded.status_table.store, FDB_SECTOR_STORE_STATUS_NUM as usize),
            FdbSectorStoreStatus::Empty as usize
        );
        assert_eq!(
            get_status(&decoded.status_table.dirty, FDB_SECTOR_DIRTY_STATUS_NUM as usize),
            FdbSectorDirtyStatus::False as usize
        );
    }

    #[test]
    fn test_kv_hdr_round_trip() {
        let mut hdr = KvHdrData::default();
        hdr.magic = KV_MAGIC_WORD;
        hdr.len = KV_HDR_DATA_SIZE + wg_align(4) + wg_align(8); // name=4, value=8
        hdr.crc32 = 0xDEAD_BEEF;
        hdr.name_len = 4;
        hdr.value_len = 8;

        let buf = hdr.to_bytes();
        let decoded = KvHdrData::from_bytes(&buf);
        assert_eq!(decoded.magic, KV_MAGIC_WORD);
        assert_eq!(decoded.len, hdr.len);
        assert_eq!(decoded.crc32, 0xDEAD_BEEF);
        assert_eq!(decoded.name_len, 4);
        assert_eq!(decoded.value_len, 8);
    }

    #[test]
    fn test_remain_threshold() {
        // c: fdb_kvdb.c:43 — FDB_SEC_REMAIN_THRESHOLD == KV_HDR_DATA_SIZE + FDB_KV_NAME_MAX
        assert_eq!(FDB_SEC_REMAIN_THRESHOLD, KV_HDR_DATA_SIZE + FDB_KV_NAME_MAX as u32);
    }

    #[test]
    fn test_magic_words() {
        // c: fdb_kvdb.c:35,37 — magic words encode ASCII 'FDB1' / 'KV00'
        assert_eq!(SECTOR_MAGIC_WORD, 0x3042_4446);
        assert_eq!(KV_MAGIC_WORD, 0x3030_564B);
    }

    // ===== T11 test helpers: manually construct on-flash KV/sector data =====

    use crate::mock_flash::MockFlash;

    fn make_kvdb() -> (FdbKvdb, MockFlash) {
        let mut db = FdbKvdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        db.parent.oldest_addr = 0;
        db.parent.init_ok = true;
        let flash = MockFlash::new("kvdb_test", 4096, 16384, 4096);
        (db, flash)
    }

    /// Build a KV header whose CRC32 matches what `read_kv` will compute.
    fn build_kv_hdr(name: &[u8], value: &[u8]) -> KvHdrData {
        let mut hdr = KvHdrData::default();
        hdr.magic = KV_MAGIC_WORD;
        hdr.name_len = name.len() as u8;
        hdr.value_len = value.len() as u32;
        hdr.len = KV_HDR_DATA_SIZE + wg_align(name.len() as u32) + wg_align(value.len() as u32);
        // CRC32(name_len[4] + value_len[4] + name + name_pad + value + value_pad)
        let buf = hdr.to_bytes();
        let mut crc = 0u32;
        crc = calc_crc32(crc, &buf[KV_NAME_LEN_OFFSET..KV_NAME_LEN_OFFSET + 4]);
        let vloff = KV_NAME_LEN_OFFSET + 4;
        crc = calc_crc32(crc, &buf[vloff..vloff + 4]);
        crc = calc_crc32(crc, name);
        let name_pad = wg_align(name.len() as u32) as usize - name.len();
        for _ in 0..name_pad {
            crc = calc_crc32(crc, &[FDB_BYTE_ERASED]);
        }
        crc = calc_crc32(crc, value);
        let value_pad = wg_align(value.len() as u32) as usize - value.len();
        for _ in 0..value_pad {
            crc = calc_crc32(crc, &[FDB_BYTE_ERASED]);
        }
        hdr.crc32 = crc;
        hdr
    }

    /// Write a complete KV to flash (status table + header + name + value).
    fn write_kv_to_flash<F: FlashDevice>(
        flash: &mut F,
        addr: u32,
        hdr: &KvHdrData,
        name: &[u8],
        value: &[u8],
        status: FdbKvStatus,
    ) {
        // write status table (c: write_kv_hdr step 1)
        let mut status_table = hdr.status_table;
        write_status(
            flash,
            addr,
            &mut status_table,
            FDB_KV_STATUS_NUM as usize,
            status as usize,
        )
        .unwrap();
        // write magic..end (c: write_kv_hdr step 2)
        let buf = hdr.to_bytes();
        flash_write(flash, addr + KV_MAGIC_OFFSET as u32, &buf[KV_MAGIC_OFFSET..]).unwrap();
        // write name (aligned)
        flash_write_align(flash, addr + KV_HDR_DATA_SIZE, name).unwrap();
        // write value (aligned)
        flash_write_align(flash, addr + KV_HDR_DATA_SIZE + wg_align(name.len() as u32), value)
            .unwrap();
    }

    /// Format a sector: erase + write sector header (magic, combined, store, dirty).
    fn format_sector_for_test<F: FlashDevice>(
        flash: &mut F,
        addr: u32,
        sec_size: u32,
        store: FdbSectorStoreStatus,
    ) {
        flash_erase(flash, addr, sec_size).unwrap();
        let mut hdr = SectorHdrData::default();
        set_status(
            &mut hdr.status_table.store,
            FDB_SECTOR_STORE_STATUS_NUM as usize,
            store as usize,
        );
        set_status(
            &mut hdr.status_table.dirty,
            FDB_SECTOR_DIRTY_STATUS_NUM as usize,
            FdbSectorDirtyStatus::False as usize,
        );
        hdr.magic = SECTOR_MAGIC_WORD;
        hdr.combined = SECTOR_NOT_COMBINED;
        hdr.reserved = FDB_DATA_UNUSED;
        let buf = hdr.to_bytes();
        flash_write(flash, addr, &buf[..SECTOR_HDR_DATA_SIZE as usize]).unwrap();
    }

    // ===== T11 tests =====

    #[test]
    fn test_read_kv() {
        // Scenario: manually write a complete KV, then read_kv parses all fields.
        let (db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let name = b"test";
        let value = b"abcd";
        let kv_addr = SECTOR_HDR_DATA_SIZE; // 16
        let hdr = build_kv_hdr(name, value);
        write_kv_to_flash(&mut flash, kv_addr, &hdr, name, value, FdbKvStatus::Write);

        let mut kv = FdbKv::default();
        kv.addr_start = kv_addr;
        let result = db.read_kv(&mut flash, &mut kv);
        assert!(result.is_ok(), "read_kv should succeed on a valid KV");
        assert!(kv.crc_is_ok, "CRC must verify for a correctly-written KV");
        assert_eq!(kv.status, FdbKvStatus::Write);
        assert_eq!(kv.name_len, 4);
        assert_eq!(&kv.name[..4], b"test");
        assert_eq!(kv.value_len, 4);
        assert_eq!(
            kv.len,
            KV_HDR_DATA_SIZE + wg_align(4) + wg_align(4)
        );
        // value address = kv_addr + KV_HDR_DATA_SIZE + wg_align(name_len)
        assert_eq!(
            kv.addr_value,
            kv_addr + KV_HDR_DATA_SIZE + wg_align(4)
        );
    }

    #[test]
    fn test_read_kv_crc_error() {
        // Scenario: a KV whose on-flash value differs from the CRC → crc_is_ok false.
        let (db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let name = b"k";
        let value = b"v1";
        let kv_addr = SECTOR_HDR_DATA_SIZE;
        let hdr = build_kv_hdr(name, value);
        write_kv_to_flash(&mut flash, kv_addr, &hdr, name, value, FdbKvStatus::Write);
        // corrupt the value byte (flip a bit so CRC no longer matches)
        let value_addr = kv_addr + KV_HDR_DATA_SIZE + wg_align(name.len() as u32);
        flash_write(&mut flash, value_addr, &[0x00]).unwrap(); // 0xFF & 0x00 = 0x00

        let mut kv = FdbKv::default();
        kv.addr_start = kv_addr;
        let result = db.read_kv(&mut flash, &mut kv);
        assert!(result.is_err(), "CRC failure should return ReadErr");
        assert!(!kv.crc_is_ok, "crc_is_ok must be false on mismatch");
    }

    #[test]
    fn test_read_kv_invalid_len_marks_err_hdr() {
        // Scenario: KV with len == u32::MAX → marked ERR_HDR on flash.
        let (db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let name = b"k";
        let value = b"v";
        let kv_addr = SECTOR_HDR_DATA_SIZE;
        let mut hdr = build_kv_hdr(name, value);
        hdr.len = u32::MAX; // invalid length
        write_kv_to_flash(&mut flash, kv_addr, &hdr, name, value, FdbKvStatus::Write);

        let mut kv = FdbKv::default();
        kv.addr_start = kv_addr;
        let result = db.read_kv(&mut flash, &mut kv);
        assert!(result.is_err(), "invalid length should return ReadErr");
        assert_eq!(kv.status, FdbKvStatus::ErrHdr);
        assert_eq!(kv.len, KV_HDR_DATA_SIZE); // reserved to header size

        // verify the ERR_HDR status was persisted to flash
        let mut status_table = [0u8; KV_STATUS_TABLE_SIZE];
        let decoded = read_status(&flash, kv_addr, &mut status_table, FDB_KV_STATUS_NUM as usize);
        assert_eq!(
            kv_status_from_index(decoded),
            FdbKvStatus::ErrHdr,
            "flash status must now read back as ERR_HDR"
        );
    }

    #[test]
    fn test_find_next_kv_addr() {
        // Scenario: two KVs in a sector; scan finds the second magic word.
        let (db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let n1 = b"k1";
        let v1 = b"v1";
        let hdr1 = build_kv_hdr(n1, v1);
        let addr1 = SECTOR_HDR_DATA_SIZE;
        write_kv_to_flash(&mut flash, addr1, &hdr1, n1, v1, FdbKvStatus::Write);

        let n2 = b"k2";
        let v2 = b"v2";
        let hdr2 = build_kv_hdr(n2, v2);
        let addr2 = addr1 + hdr1.len;
        write_kv_to_flash(&mut flash, addr2, &hdr2, n2, v2, FdbKvStatus::Write);

        // scan from the end of KV1; should find KV2's start address
        let end = 4096 - SECTOR_HDR_DATA_SIZE;
        let found = db.find_next_kv_addr(&flash, addr1 + hdr1.len, end);
        assert_eq!(found, addr2, "find_next_kv_addr should locate KV2");
    }

    #[test]
    fn test_find_next_kv_addr_none() {
        // Scenario: no more magic words after the given start → FAILED_ADDR.
        let (db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let end = 4096 - SECTOR_HDR_DATA_SIZE;
        let found = db.find_next_kv_addr(&flash, 4060, end);
        assert_eq!(found, FDB_FAILED_ADDR);
    }

    #[test]
    fn test_kv_iterator() {
        // Scenario: iterate all valid WRITE KVs across the sector.
        let (mut db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let n1 = b"alpha";
        let v1 = b"one";
        let hdr1 = build_kv_hdr(n1, v1);
        let addr1 = SECTOR_HDR_DATA_SIZE;
        write_kv_to_flash(&mut flash, addr1, &hdr1, n1, v1, FdbKvStatus::Write);

        let n2 = b"beta";
        let v2 = b"two";
        let hdr2 = build_kv_hdr(n2, v2);
        let addr2 = addr1 + hdr1.len;
        write_kv_to_flash(&mut flash, addr2, &hdr2, n2, v2, FdbKvStatus::Write);

        let mut found_names: Vec<u8> = Vec::new();
        let mut kv = FdbKv::default();
        db.kv_iterator(&mut flash, &mut kv, |_flash, cur_kv| {
            if cur_kv.crc_is_ok && cur_kv.status == FdbKvStatus::Write {
                found_names.extend_from_slice(&cur_kv.name[..cur_kv.name_len as usize]);
                found_names.push(b',');
            }
            false // keep iterating
        });
        let s = String::from_utf8(found_names.clone()).unwrap();
        assert!(
            s.contains("alpha") && s.contains("beta"),
            "iterator should visit both KVs, got: {s}"
        );
    }

    #[test]
    fn test_find_kv() {
        // Scenario: find_kv locates a KV by name (no cache feature in default build).
        let (mut db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let name = b"mykey";
        let value = b"myval";
        let hdr = build_kv_hdr(name, value);
        let kv_addr = SECTOR_HDR_DATA_SIZE;
        write_kv_to_flash(&mut flash, kv_addr, &hdr, name, value, FdbKvStatus::Write);

        let mut kv = FdbKv::default();
        let found = db.find_kv(&mut flash, name, &mut kv);
        assert!(found, "find_kv should locate the written KV");
        assert_eq!(&kv.name[..name.len()], name);
        assert_eq!(kv.addr_start, kv_addr);
        assert_eq!(kv.value_len, value.len() as u32);
    }

    #[test]
    fn test_find_kv_not_found() {
        let (mut db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let name = b"present";
        let value = b"v";
        let hdr = build_kv_hdr(name, value);
        write_kv_to_flash(&mut flash, SECTOR_HDR_DATA_SIZE, &hdr, name, value, FdbKvStatus::Write);

        let mut kv = FdbKv::default();
        let found = db.find_kv(&mut flash, b"absent", &mut kv);
        assert!(!found, "find_kv should return false for a missing key");
    }

    #[test]
    fn test_get_kv() {
        // Scenario: get_kv reads the value bytes and reports value_len.
        let (mut db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let name = b"key";
        let value = b"hello";
        let hdr = build_kv_hdr(name, value);
        write_kv_to_flash(&mut flash, SECTOR_HDR_DATA_SIZE, &hdr, name, value, FdbKvStatus::Write);

        let mut buf = [0u8; 16];
        let mut vlen = 0usize;
        let read_len = db.get_kv(&mut flash, name, &mut buf, Some(&mut vlen));
        assert_eq!(read_len, 5);
        assert_eq!(vlen, 5);
        assert_eq!(&buf[..5], b"hello");
    }

    #[test]
    fn test_get_kv_truncated_buffer() {
        // Scenario: buffer smaller than value → read_len == buf_len, value_len == full.
        let (mut db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let name = b"k";
        let value = b"abcdef";
        let hdr = build_kv_hdr(name, value);
        write_kv_to_flash(&mut flash, SECTOR_HDR_DATA_SIZE, &hdr, name, value, FdbKvStatus::Write);

        let mut buf = [0u8; 3];
        let mut vlen = 0usize;
        let read_len = db.get_kv(&mut flash, name, &mut buf, Some(&mut vlen));
        assert_eq!(read_len, 3, "read truncated to buffer size");
        assert_eq!(vlen, 6, "value_len reports full length");
        assert_eq!(&buf[..3], b"abc");
    }

    #[test]
    fn test_fdb_is_str() {
        // c: fdb_kvdb.c:609-620 — printable ASCII check
        assert!(fdb_is_str(b"hello world"));
        assert!(fdb_is_str(b"123"));
        assert!(fdb_is_str(b"")); // empty is vacuously true
        assert!(!fdb_is_str(&[0x00]), "NUL is not printable");
        assert!(!fdb_is_str(&[0x1F]), "unit separator below space is not printable");
        assert!(!fdb_is_str(&[0x7F]), "DEL is not printable");
        assert!(fdb_is_str(&[0x7E]), "~ (0x7E) is printable");
    }

    #[test]
    fn test_get_next_sector_addr() {
        let (db, _flash) = make_kvdb();
        // sector 0 → sector 1
        let mut sec = KvdbSecInfo::default();
        sec.addr = 0;
        sec.combined = SECTOR_NOT_COMBINED;
        assert_eq!(db.get_next_sector_addr(&sec, 4096), 4096);
        // last sector (addr 12288, sec_size 4096, max 16384) → wrap to 0
        sec.addr = 12288;
        assert_eq!(db.get_next_sector_addr(&sec, 12288), 0);
        // traversed all → FAILED_ADDR
        sec.addr = 12288;
        assert_eq!(db.get_next_sector_addr(&sec, 16384), FDB_FAILED_ADDR);
    }

    #[test]
    fn test_read_sector_info_empty() {
        let (mut db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Empty);
        let mut sector = KvdbSecInfo::default();
        let result = db.read_sector_info(&mut flash, 0, &mut sector, true);
        assert!(result.is_ok());
        assert!(sector.check_ok);
        assert_eq!(sector.store, FdbSectorStoreStatus::Empty);
        assert_eq!(sector.magic, SECTOR_MAGIC_WORD);
        assert_eq!(sector.combined, SECTOR_NOT_COMBINED);
        // empty sector: remain = sec_size - SECTOR_HDR_DATA_SIZE
        assert_eq!(sector.remain, (4096 - SECTOR_HDR_DATA_SIZE) as usize);
        assert_eq!(sector.empty_kv, SECTOR_HDR_DATA_SIZE);
    }

    #[test]
    fn test_read_sector_info_bad_magic() {
        let (mut db, mut flash) = make_kvdb();
        // erased sector (no header) → magic 0xFFFFFFFF → InitFailed
        let mut sector = KvdbSecInfo::default();
        let result = db.read_sector_info(&mut flash, 0, &mut sector, false);
        assert_eq!(result, Err(FdbErr::InitFailed));
        assert!(!sector.check_ok);
    }

    // ===== T12 tests: CRUD cycle, two-phase write, blob, GC =====

    fn init_kvdb_for_crud(db: &mut FdbKvdb, flash: &mut MockFlash) {
        db.parent.init_ok = true;
        db.parent.oldest_addr = 0;
        let max_size = db.parent.max_size;
        let sec_size = db.parent.sec_size;
        let mut addr = 0u32;
        while addr < max_size {
            db.format_sector(flash, addr, SECTOR_NOT_COMBINED).unwrap();
            addr += sec_size;
        }
    }

    fn make_kvdb_2sec() -> (FdbKvdb, MockFlash) {
        let mut db = FdbKvdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 8192; // 2 sectors
        db.parent.oldest_addr = 0;
        db.parent.init_ok = true;
        let flash = MockFlash::new("kvdb_test", 4096, 8192, 4096);
        (db, flash)
    }

    #[test]
    fn test_kv_crud_cycle() {
        // Scenario: set → get → update → get → del → get → verify deleted.
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);

        // set
        db.kv_set(&mut flash, "key1", "value1").unwrap();
        // get
        assert_eq!(db.kv_get(&mut flash, "key1"), Some("value1".to_string()));
        // update
        db.kv_set(&mut flash, "key1", "value2").unwrap();
        assert_eq!(db.kv_get(&mut flash, "key1"), Some("value2".to_string()));
        // del
        db.kv_del(&mut flash, "key1").unwrap();
        assert_eq!(db.kv_get(&mut flash, "key1"), None, "deleted KV must not be found");
    }

    #[test]
    fn test_two_phase_write() {
        // Scenario: after kv_set the on-flash KV status is WRITE (PRE_WRITE→WRITE).
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);

        db.kv_set(&mut flash, "k", "v").unwrap();
        let mut kv = FdbKv::default();
        assert!(db.kv_get_obj(&mut flash, "k", &mut kv));
        assert_eq!(kv.status, FdbKvStatus::Write, "two-phase write ends in WRITE");
        assert!(kv.crc_is_ok);
        assert_eq!(kv.value_len, 1);
    }

    #[test]
    fn test_blob_set_get() {
        // Scenario: binary blob round-trip (non-string data).
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);

        let data: Vec<u8> = (0x01..=0x20).collect();
        let mut write_buf = data.clone();
        let mut blob = blob_make(&mut write_buf);
        db.kv_set_blob(&mut flash, "bin", &mut blob).unwrap();

        let mut read_buf = vec![0u8; 32];
        let mut blob2 = blob_make(&mut read_buf);
        let read = db.kv_get_blob(&mut flash, "bin", &mut blob2);
        assert_eq!(read, Some(32));
        assert_eq!(&read_buf[..32], &data[..], "blob data must round-trip exactly");
    }

    #[test]
    fn test_kv_set_multiple_keys() {
        // Scenario: multiple distinct keys coexist and are independently readable.
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);

        db.kv_set(&mut flash, "alpha", "1").unwrap();
        db.kv_set(&mut flash, "beta", "2").unwrap();
        db.kv_set(&mut flash, "gamma", "3").unwrap();

        assert_eq!(db.kv_get(&mut flash, "alpha"), Some("1".to_string()));
        assert_eq!(db.kv_get(&mut flash, "beta"), Some("2".to_string()));
        assert_eq!(db.kv_get(&mut flash, "gamma"), Some("3".to_string()));
    }

    #[test]
    fn test_kv_get_not_found() {
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);
        assert_eq!(db.kv_get(&mut flash, "absent"), None);
    }

    #[test]
    fn test_kv_get_blob_reports_value_len() {
        // Scenario: small buffer, value_len reports the full stored length.
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);

        db.kv_set(&mut flash, "k", "abcdef").unwrap();
        let mut buf = [0u8; 3];
        let mut blob = blob_make(&mut buf);
        let read = db.kv_get_blob(&mut flash, "k", &mut blob);
        assert_eq!(read, Some(3), "read truncated to buffer size");
        assert_eq!(blob.saved_len, 6, "saved_len reports full value length");
        assert_eq!(&buf[..3], b"abc");
    }

    #[test]
    fn test_kv_set_empty_value() {
        // Boundary: empty string value.
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);

        db.kv_set(&mut flash, "empty", "").unwrap();
        // kv_get returns None for a 0-length value (matches C fdb_kv_get NULL for get_size==0)
        assert_eq!(db.kv_get(&mut flash, "empty"), None);
        // but the KV object exists
        let mut kv = FdbKv::default();
        assert!(db.kv_get_obj(&mut flash, "empty", &mut kv));
        assert_eq!(kv.value_len, 0);
    }

    #[test]
    fn test_kv_update_same_key() {
        // Scenario: updating the same key multiple times keeps only the latest.
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);

        db.kv_set(&mut flash, "counter", "one").unwrap();
        db.kv_set(&mut flash, "counter", "two").unwrap();
        db.kv_set(&mut flash, "counter", "three").unwrap();
        assert_eq!(db.kv_get(&mut flash, "counter"), Some("three".to_string()));
    }

    #[test]
    fn test_kv_del_absent_key() {
        // Error path: deleting a non-existent key returns KvNameErr.
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);
        let result = db.kv_del(&mut flash, "nope");
        assert_eq!(result, Err(FdbErr::KvNameErr));
    }

    #[test]
    fn test_gc_preserves_valid_kv() {
        // Scenario: after deleting a KV and triggering GC, the surviving KV is
        // moved to a fresh sector and remains readable.
        let (mut db, mut flash) = make_kvdb_2sec();
        init_kvdb_for_crud(&mut db, &mut flash);

        db.kv_set(&mut flash, "survive", "keepme").unwrap();
        db.kv_set(&mut flash, "doomed", "deleteme").unwrap();
        assert_eq!(db.kv_get(&mut flash, "survive"), Some("keepme".to_string()));
        assert_eq!(db.kv_get(&mut flash, "doomed"), Some("deleteme".to_string()));

        // delete doomed (marks its sector dirty)
        db.kv_del(&mut flash, "doomed").unwrap();
        assert_eq!(db.kv_get(&mut flash, "doomed"), None);

        // trigger GC (gc_request must be true so alloc_kv uses empty sectors)
        db.gc_request = true;
        db.gc_collect(&mut flash);

        // survive must still be readable (moved to the other sector)
        assert_eq!(
            db.kv_get(&mut flash, "survive"),
            Some("keepme".to_string()),
            "GC must preserve valid KVs"
        );
        assert_eq!(db.kv_get(&mut flash, "doomed"), None);

        // can still write new KVs after GC (sector was reformatted)
        db.kv_set(&mut flash, "fresh", "newval").unwrap();
        assert_eq!(db.kv_get(&mut flash, "fresh"), Some("newval".to_string()));
    }

    #[test]
    fn test_kv_to_blob() {
        // Scenario: kv_to_blob wires a KV object into a blob for subsequent read.
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);
        db.kv_set(&mut flash, "k", "hello").unwrap();

        let mut kv = FdbKv::default();
        assert!(db.kv_get_obj(&mut flash, "k", &mut kv));

        let mut buf = [0u8; 16];
        let mut blob = blob_make(&mut buf);
        kv_to_blob(&kv, &mut blob);
        assert_eq!(blob.saved_meta_addr, kv.addr_start);
        assert_eq!(blob.saved_addr, kv.addr_value);
        assert_eq!(blob.saved_len, 5);

        // use the blob to read the value directly from flash
        let n = blob_read(&flash, &mut blob);
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], b"hello");
    }

    // ===== T13 tests: init, set_default, recovery, iterator, check =====

    #[test]
    fn test_kvdb_init_empty_flash() {
        // Scenario: init on totally erased flash → all sector headers bad → set_default.
        let mut db = FdbKvdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let default_kvs = FdbDefaultKv { kvs: &[] };
        let result = db.kvdb_init(&mut flash, "kvdb", "part", default_kvs);
        assert!(result.is_ok(), "init on empty flash should succeed");
        assert!(db.db_init_ok());
        // CRUD works after init
        db.kv_set(&mut flash, "k", "v").unwrap();
        assert_eq!(db.kv_get(&mut flash, "k"), Some("v".to_string()));
    }

    #[test]
    fn test_kvdb_init_with_defaults() {
        // Scenario: init with default KVs → defaults are created on empty flash.
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
        let mut db = FdbKvdb::default();
        db.parent.sec_size = 4096;
        db.parent.max_size = 16384;
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        db.kvdb_init(
            &mut flash,
            "kvdb",
            "part",
            FdbDefaultKv { kvs: DEFAULT_KVS },
        )
        .unwrap();
        assert_eq!(
            db.kv_get(&mut flash, "boot_count"),
            Some("0".to_string())
        );
        assert_eq!(db.kv_get(&mut flash, "version"), Some("1.0".to_string()));
    }

    #[test]
    fn test_kv_set_default() {
        // Scenario: set_default wipes custom KVs and restores defaults.
        static DEFAULT_KVS: &[FdbDefaultKvNode] = &[
            FdbDefaultKvNode {
                key: "dk1",
                value: b"dv1",
                value_len: 0,
            },
            FdbDefaultKvNode {
                key: "dk2",
                value: &[0x01, 0x02],
                value_len: 2,
            },
        ];
        let (mut db, mut flash) = make_kvdb();
        db.default_kvs = FdbDefaultKv { kvs: DEFAULT_KVS };
        init_kvdb_for_crud(&mut db, &mut flash);
        db.kv_set(&mut flash, "custom", "x").unwrap();
        assert_eq!(db.kv_get(&mut flash, "custom"), Some("x".to_string()));

        db.kv_set_default(&mut flash).unwrap();
        assert_eq!(
            db.kv_get(&mut flash, "custom"),
            None,
            "set_default wipes custom KVs"
        );
        assert_eq!(db.kv_get(&mut flash, "dk1"), Some("dv1".to_string()));
        // dk2 is binary (not a string) → kv_get returns None, but the KV object exists
        let mut kv = FdbKv::default();
        assert!(db.kv_get_obj(&mut flash, "dk2", &mut kv));
        assert_eq!(kv.value_len, 2);
    }

    #[test]
    fn test_recovery_pre_write() {
        // Scenario: a PRE_WRITE KV (interrupted write) is marked ERR_HDR on load.
        let (mut db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let name = b"interrupted";
        let value = b"data";
        let hdr = build_kv_hdr(name, value);
        let kv_addr = SECTOR_HDR_DATA_SIZE;
        write_kv_to_flash(&mut flash, kv_addr, &hdr, name, value, FdbKvStatus::PreWrite);

        db.parent.oldest_addr = 0;
        db.parent.init_ok = true;
        let result = db.fdb_kv_load(&mut flash);
        assert!(result.is_ok());

        // the PRE_WRITE KV should now be marked ERR_HDR on flash
        let mut status_table = [0u8; KV_STATUS_TABLE_SIZE];
        let decoded = read_status(&flash, kv_addr, &mut status_table, FDB_KV_STATUS_NUM as usize);
        assert_eq!(
            kv_status_from_index(decoded),
            FdbKvStatus::ErrHdr,
            "PRE_WRITE KV must be recovered to ERR_HDR"
        );
        // and it's not readable as a valid KV
        assert_eq!(db.kv_get(&mut flash, "interrupted"), None);
    }

    #[test]
    fn test_recovery_pre_delete() {
        // Scenario: a PRE_DELETE KV (interrupted delete) is recovered (moved to WRITE).
        let (mut db, mut flash) = make_kvdb();
        format_sector_for_test(&mut flash, 0, 4096, FdbSectorStoreStatus::Using);
        let name = b"recoverme";
        let value = b"val";
        let hdr = build_kv_hdr(name, value);
        let kv_addr = SECTOR_HDR_DATA_SIZE;
        write_kv_to_flash(&mut flash, kv_addr, &hdr, name, value, FdbKvStatus::PreDelete);

        db.parent.oldest_addr = 0;
        db.parent.init_ok = true;
        db.gc_request = false;
        let result = db.fdb_kv_load(&mut flash);
        assert!(result.is_ok());

        // the PRE_DELETE KV should be recovered (moved to new space as WRITE)
        assert_eq!(
            db.kv_get(&mut flash, "recoverme"),
            Some("val".to_string()),
            "PRE_DELETE KV must be recovered"
        );
    }

    #[test]
    fn test_kvdb_iterator() {
        // Scenario: iterator visits all valid WRITE KVs.
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);
        db.kv_set(&mut flash, "a", "1").unwrap();
        db.kv_set(&mut flash, "b", "2").unwrap();
        db.kv_set(&mut flash, "c", "3").unwrap();

        let mut itr = db.kv_iterator_init();
        let mut count = 0;
        let mut keys: Vec<String> = Vec::new();
        while db.kv_iterate(&mut flash, &mut itr) {
            count += 1;
            keys.push(itr.curr_kv.name_str().to_string());
        }
        assert_eq!(count, 3, "iterator must visit all 3 KVs");
        assert!(keys.contains(&"a".to_string()));
        assert!(keys.contains(&"b".to_string()));
        assert!(keys.contains(&"c".to_string()));
    }

    #[test]
    fn test_kvdb_check_healthy() {
        // Scenario: integrity check passes on a healthy db.
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);
        db.kv_set(&mut flash, "k", "v").unwrap();
        db.kv_set(&mut flash, "k2", "v2").unwrap();
        let result = db.kvdb_check(&mut flash);
        assert!(result.is_ok(), "check should pass on a healthy db");
    }

    #[test]
    fn test_kvdb_deinit() {
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);
        assert!(db.db_init_ok());
        db.kvdb_deinit().unwrap();
        assert!(!db.db_init_ok(), "deinit must clear init_ok");
        // operations after deinit fail
        assert_eq!(db.kv_get(&mut flash, "k"), None);
    }

    #[test]
    fn test_kv_print() {
        // Scenario: kv_print produces name=value lines for string KVs.
        let (mut db, mut flash) = make_kvdb();
        init_kvdb_for_crud(&mut db, &mut flash);
        db.kv_set(&mut flash, "name", "flashdb").unwrap();
        db.kv_set(&mut flash, "ver", "2").unwrap();
        let output = db.kv_print(&mut flash);
        assert!(output.contains("name=flashdb"), "print must show name=value");
        assert!(output.contains("ver=2"));
        assert!(output.contains("mode: next generation"), "print must show mode line");
    }

    #[test]
    fn test_control_setters() {
        // Scenario: builder setters configure the db before init.
        let mut db = FdbKvdb::default();
        db.set_sec_size(4096);
        assert_eq!(db.get_sec_size(), 4096);
        db.parent.max_size = 16384;
        // set_lock installs a callback (fn pointer, no captured state)
        db.set_lock(|_db| {}); // no-op lock
        db.set_unlock(|_db| {});
        db.set_not_format(true);
        assert!(db.parent.not_formatable);
    }
}
