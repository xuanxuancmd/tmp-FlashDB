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

use alloc::string::String;
use alloc::vec::Vec;

use crate::def::{
    FdbBlob, FdbDb, FdbDefaultKv, FdbErr, FdbKv, FdbKvIterator, FdbKvStatus, FdbKvdb,
    FdbSectorDirtyStatus, FdbSectorStoreStatus, KvdbSecInfo, FDB_BYTE_ERASED, FDB_DATA_UNUSED,
    FDB_DIRTY_STATUS_TABLE_SIZE, FDB_FAILED_ADDR, FDB_KV_NAME_MAX, FDB_KV_STATUS_NUM,
    FDB_SECTOR_DIRTY_STATUS_NUM, FDB_SECTOR_STORE_STATUS_NUM, FDB_STORE_STATUS_TABLE_SIZE,
};
use crate::flash_trait::FlashDevice;
use crate::low_lvl::{
    align_down, calc_crc32, continue_ff_addr, flash_erase, flash_read, flash_write,
    flash_write_align, get_status, read_status, set_status, status_table_size, wg_align,
    write_status,
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
#[derive(Clone, Copy)]
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
    #[cfg(any(feature = "gran_64", feature = "gran_128"))]
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

impl Default for SectorHdrData {
    fn default() -> Self {
        Self {
            status_table: SectorStatusTable::default(),
            magic: 0,
            combined: 0,
            reserved: 0,
            #[cfg(any(feature = "gran_64", feature = "gran_128"))]
            padding: [0; 4],
            #[cfg(feature = "gran_256")]
            padding: [0; 20],
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
    #[cfg(feature = "gran_64")]
    pub padding: [u8; 4],
    /// c: fdb_kvdb.c:127-128 — align padding for 128bit write granularity
    #[cfg(feature = "gran_128")]
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
            #[cfg(feature = "gran_64")]
            padding: [0; 4],
            #[cfg(feature = "gran_128")]
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

#[cfg(feature = "gran_8")]
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == 20);
#[cfg(feature = "gran_8")]
const _: () = assert!(core::mem::size_of::<KvHdrData>() == 28);

#[cfg(feature = "gran_32")]
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == 36);
#[cfg(feature = "gran_32")]
const _: () = assert!(core::mem::size_of::<KvHdrData>() == 40);

#[cfg(feature = "gran_64")]
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == 64);
#[cfg(feature = "gran_64")]
const _: () = assert!(core::mem::size_of::<KvHdrData>() == 64);

#[cfg(feature = "gran_128")]
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == 112);
#[cfg(feature = "gran_128")]
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
    pub(crate) fn to_bytes(&self) -> [u8; SECTOR_HDR_SIZE] {
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
    pub(crate) fn to_bytes(&self) -> [u8; KV_HDR_SIZE] {
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
    pub(crate) fn get_kv_from_cache<F: FlashDevice>(
        &self,
        flash: &F,
        name: &[u8],
    ) -> Option<u32> {
        use crate::def::FDB_KV_CACHE_TABLE_SIZE;
        let name_crc = (calc_crc32(0, name) >> 16) as u16;
        for i in 0..FDB_KV_CACHE_TABLE_SIZE {
            let entry = &self.kv_cache_table[i];
            if entry.addr != FDB_DATA_UNUSED && entry.name_crc == name_crc {
                let mut saved_name = [0u8; FDB_KV_NAME_MAX];
                let _ = flash_read(
                    flash,
                    entry.addr + KV_HDR_DATA_SIZE,
                    &mut saved_name,
                );
                if &saved_name[..name.len()] == name {
                    let new_active = if entry.active >= 0xFFFF - FDB_KV_CACHE_TABLE_SIZE as u16 {
                        0xFFFF
                    } else {
                        entry.active + FDB_KV_CACHE_TABLE_SIZE as u16
                    };
                    // need &mut to bump active; reborrow via index on self
                    self.bump_kv_cache_active(i, new_active);
                    return Some(entry.addr);
                }
            }
        }
        None
    }

    /// Helper to bump a cache entry's active counter (borrows &mut self at the index).
    fn bump_kv_cache_active(&mut self, index: usize, new_active: u16) {
        use crate::def::FDB_KV_CACHE_TABLE_SIZE;
        if index < FDB_KV_CACHE_TABLE_SIZE {
            self.kv_cache_table[index].active = new_active;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
