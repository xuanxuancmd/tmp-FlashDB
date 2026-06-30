// c: flashdb.h — Public API crate root
#![cfg_attr(not(test), no_std)]
#![allow(clippy::missing_safety_doc)]

// KVDB public APIs (fdb_kv_get -> Option<String>, fdb_kv_print -> String) need
// dynamic allocation. `extern crate alloc` works in both std (tests) and no_std
// (final binary supplies a global allocator).
extern crate alloc;

pub mod def;
pub mod flash_trait;
pub mod init;
pub mod kvdb;
pub mod low_lvl;
#[cfg(feature = "tsdb")]
pub mod tsdb;

pub mod mock_flash;

pub use mock_flash::MockFlash;

pub use def::{
    FdbBlob, FdbDb, FdbDbType, FdbDefaultKv, FdbDefaultKvNode, FdbErr, FdbGetTime, FdbKv,
    FdbKvIterator, FdbKvStatus, FdbSectorDirtyStatus, FdbSectorStoreStatus, FdbTsl, FdbTslStatus,
    FdbTime, FdbTsdb, FdbKvdb, KvCacheNode, KvdbSecInfo, TsdbSecInfo,
    FDB_BYTE_ERASED, FDB_DATA_UNUSED, FDB_FAILED_ADDR, FDB_KV_STATUS_NUM,
    FDB_SECTOR_DIRTY_STATUS_NUM, FDB_SECTOR_STORE_STATUS_NUM, FDB_TSL_STATUS_NUM,
};
pub use flash_trait::FlashDevice;
pub use init::{db_path, deinit, init_ex, init_finish};
pub use kvdb::{kv_to_blob, SECTOR_DIRTY_OFFSET, SECTOR_MAGIC_OFFSET};
pub use low_lvl::{
    blob_make, blob_read, calc_crc32, continue_ff_addr, flash_erase, flash_read, flash_write,
    flash_write_align, get_status, read_status, set_status, write_status,
};

pub const FDB_SW_VERSION: &str = "2.2.99";
pub const FDB_SW_VERSION_NUM: u32 = 0x20299;
