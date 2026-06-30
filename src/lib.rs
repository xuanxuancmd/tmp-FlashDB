// c: flashdb.h — Public API crate root
#![cfg_attr(not(test), no_std)]
#![allow(clippy::missing_safety_doc)]

pub mod def;
pub mod flash_trait;
pub mod init;
pub mod low_lvl;
#[cfg(feature = "tsdb")]
pub mod tsdb;

#[cfg(test)]
pub mod mock_flash;

pub use def::{
    FdbBlob, FdbDb, FdbDbType, FdbDefaultKv, FdbDefaultKvNode, FdbErr, FdbKv, FdbKvIterator,
    FdbKvStatus, FdbSectorDirtyStatus, FdbSectorStoreStatus, FdbTsl, FdbTslStatus, FdbTime,
    FdbTsdb, FdbKvdb, KvCacheNode, KvdbSecInfo, TsdbSecInfo,
};
pub use flash_trait::FlashDevice;
pub use init::{db_path, deinit, init_ex, init_finish};
pub use low_lvl::{
    blob_make, blob_read, calc_crc32, continue_ff_addr, flash_erase, flash_read, flash_write,
    flash_write_align, get_status, read_status, set_status, write_status,
};

pub const FDB_SW_VERSION: &str = "2.2.99";
pub const FDB_SW_VERSION_NUM: u32 = 0x20299;
