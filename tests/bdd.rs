// tests/bdd.rs — BDD cucumber-rust main entry point + World definition
//
// Registers all step definition modules and runs every .feature file in
// tests/features/.  The FlashDB library itself is no_std; these tests run in
// the std environment via the `mock_flash` module (compiled under cfg(test)).

#[path = "bdd/kvdb_steps.rs"]
mod kvdb_steps;
#[path = "bdd/tsdb_steps.rs"]
mod tsdb_steps;

/// Inline split-borrow of `world.flash` — avoids borrowing all of `world`
/// so that `world.kvdb` / `world.tsdb` can be mutably borrowed simultaneously.
macro_rules! flash_mut {
    ($world:expr) => {
        match $world.flash.as_mut() {
            Some(f) => f,
            None => panic!("MockFlash not initialised — missing a Given step"),
        }
    };
}
pub(crate) use flash_mut;

use std::sync::atomic::{AtomicI64, Ordering};

use cucumber::World;
use flashdb::{
    FdbDefaultKv, FdbDefaultKvNode, FdbErr, FdbKvIterator, FdbKvdb, FdbTsdb, FdbTime, FlashDevice,
};
use flashdb::mock_flash::MockFlash;

// ===========================================================================
// get_time callback bridge
// ===========================================================================
//
// `FdbGetTime` is a `fn() -> FdbTime` pointer — it cannot capture environment.
// TSDB scenarios need to control the timestamp returned by the callback, so we
// bridge through a global atomic.  This is safe in the test harness because
// scenarios run sequentially.

static GET_TIME_VALUE: AtomicI64 = AtomicI64::new(0);

/// The actual callback handed to `FdbTsdb::init`.
pub fn get_time_callback() -> FdbTime {
    GET_TIME_VALUE.load(Ordering::SeqCst) as FdbTime
}

/// Steps call this to program the next timestamp(s) the callback will return.
pub fn set_get_time(value: FdbTime) {
    GET_TIME_VALUE.store(value as i64, Ordering::SeqCst);
}

// ===========================================================================
// Pre-defined default KV nodes
// ===========================================================================

/// Default KV: "hostname" = "sensor-01" (used by kvdb-init.feature)
pub static DEFAULT_KV_HOSTNAME: FdbDefaultKvNode = FdbDefaultKvNode {
    key: "hostname",
    value: b"sensor-01",
    value_len: 9,
};

/// Static slice containing the hostname default KV.
static DEFAULT_KVS_HOSTNAME: &[FdbDefaultKvNode] = &[DEFAULT_KV_HOSTNAME];

/// Build a `FdbDefaultKv` slice from the hostname default.
pub fn default_kvs_with_hostname() -> FdbDefaultKv {
    FdbDefaultKv {
        kvs: DEFAULT_KVS_HOSTNAME,
    }
}

/// Empty default KV collection.
pub fn empty_default_kvs() -> FdbDefaultKv {
    FdbDefaultKv { kvs: &[] }
}

// ===========================================================================
// Leak helper
// ===========================================================================

/// Convert a runtime `String` into `&'static str` by leaking the allocation.
pub fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

// ===========================================================================
// World
// ===========================================================================

/// BDD world — one fresh instance per scenario.
#[derive(World)]
pub struct FlashWorld {
    // ---- flash backend ----
    pub flash: Option<MockFlash>,

    // ---- KVDB ----
    pub kvdb: FdbKvdb,
    pub kvdb_iterator: Option<FdbKvIterator>,
    pub iterated_kv_names: Vec<String>,
    pub iterated_kv_values: Vec<Vec<u8>>,
    pub print_output: String,

    // ---- TSDB ----
    pub tsdb: FdbTsdb,
    pub iterated_tsl_times: Vec<FdbTime>,
    pub iterated_tsl_data: Vec<Vec<u8>>,
    pub tsl_query_count_result: usize,
    pub tsl_max_blob_count_result: usize,

    // ---- shared intermediate state ----
    pub last_result: Option<Result<(), FdbErr>>,
    pub last_kv_string: Option<String>,
    pub last_kv_found: bool,
    pub last_blob_read_len: usize,
    pub blob_buf: Vec<u8>,
    pub last_panicked: bool,
}

impl Default for FlashWorld {
    fn default() -> Self {
        Self {
            flash: None,
            kvdb: FdbKvdb::default(),
            kvdb_iterator: None,
            iterated_kv_names: Vec::new(),
            iterated_kv_values: Vec::new(),
            print_output: String::new(),
            tsdb: FdbTsdb::default(),
            iterated_tsl_times: Vec::new(),
            iterated_tsl_data: Vec::new(),
            tsl_query_count_result: 0,
            tsl_max_blob_count_result: 0,
            last_result: None,
            last_kv_string: None,
            last_kv_found: false,
            last_blob_read_len: 0,
            blob_buf: Vec::new(),
            last_panicked: false,
        }
    }
}

impl std::fmt::Debug for FlashWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlashWorld")
            .field("kvdb_init_ok", &self.kvdb.parent.init_ok)
            .field("tsdb_init_ok", &self.tsdb.parent.init_ok)
            .field("last_result", &self.last_result)
            .field("last_kv_string", &self.last_kv_string)
            .field("last_blob_read_len", &self.last_blob_read_len)
            .field("iterated_kv_names", &self.iterated_kv_names)
            .field("iterated_tsl_times", &self.iterated_tsl_times)
            .field("last_panicked", &self.last_panicked)
            .finish()
    }
}

impl FlashWorld {
    /// Borrow the flash device mutably (panics if Given-step hasn't run).
    pub fn flash_mut(&mut self) -> &mut MockFlash {
        match self.flash.as_mut() {
            Some(f) => f,
            None => panic!("MockFlash not initialised — missing a Given step"),
        }
    }

    /// Initialise a KVDB on a fresh 16 KiB / 4 KiB-sector mock flash.
    pub fn setup_kvdb(&mut self, default_kvs: FdbDefaultKv) {
        self.flash = Some(MockFlash::new("fdb_kvdb1", 4096, 16384, 4096));
        self.kvdb = FdbKvdb::default();
        self.kvdb.set_sec_size(4096);
        self.kvdb.parent.max_size = 16384;
        let result = {
            let flash = flash_mut!(self);
            self.kvdb.kvdb_init(flash, "config", "fdb_kvdb1", default_kvs)
        };
        self.last_result = Some(result);
    }

    /// Initialise a TSDB on a fresh mock flash.
    pub fn setup_tsdb(&mut self, sec_size: u32, max_size: u32, max_len: usize) {
        self.flash = Some(MockFlash::new("fdb_tsdb1", sec_size, max_size, sec_size));
        self.tsdb = FdbTsdb::default();
        self.tsdb.set_sec_size(sec_size);
        self.tsdb.parent.max_size = max_size;
        self.tsdb.max_len = max_len;
        set_get_time(0);
        let result = {
            let flash = flash_mut!(self);
            self.tsdb.init(flash, "logdb", "fdb_tsdb1", get_time_callback, max_len)
        };
        self.last_result = Some(result);
    }

    /// Map a feature-file error-code name to `FdbErr`.
    pub fn parse_err(name: &str) -> FdbErr {
        match name.trim() {
            "FDB_NO_ERR" => FdbErr::NoErr,
            "FDB_READ_ERR" => FdbErr::ReadErr,
            "FDB_WRITE_ERR" => FdbErr::WriteErr,
            "FDB_ERASE_ERR" => FdbErr::EraseErr,
            "FDB_PART_NOT_FOUND" => FdbErr::PartNotFound,
            "FDB_KV_NAME_ERR" => FdbErr::KvNameErr,
            "FDB_KV_NAME_EXIST" => FdbErr::KvNameExist,
            "FDB_SAVED_FULL" => FdbErr::SavedFull,
            "FDB_INIT_FAILED" => FdbErr::InitFailed,
            other => panic!("unknown error code in feature: {}", other),
        }
    }
}

// ===========================================================================
// Entry point
// ===========================================================================

#[tokio::main]
async fn main() {
    // c: note — cucumber-rs runs scenarios CONCURRENTLY by default (up to 64).
    // Our `FdbGetTime` callback bridges through a global atomic because the
    // `fn() -> FdbTime` pointer cannot capture per-World state.  Concurrent
    // execution would race on that atomic, so we force sequential execution.
    FlashWorld::cucumber()
        .max_concurrent_scenarios(1)
        .run_and_exit("tests/features")
        .await;
}
