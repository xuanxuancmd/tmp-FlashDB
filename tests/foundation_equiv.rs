// c-port: foundation_equiv.rs — C-equivalence integration test
//
// Cross-checks the Rust Foundation layer against golden values taken directly
// from the C source (fdb_utils.c, fdb_def.h). These values are the C author's
// verified reference behaviour and exist to catch translation drift:
//   * CRC32 must match the standard CRC-32/ISO-HDLC table in fdb_utils.c:21-66.
//   * Status-table encodings match the comment table in fdb_utils.c:91-107.
//   * blob make/read round-trips against a NOR-flash backend.
//
// The library's own `MockFlash` is `#[cfg(test)]`-gated and therefore invisible
// to integration tests (a separate crate), so this file provides its own
// minimal NOR-flash simulation implementing `FlashDevice`.

use flashdb::def::{FdbBlob, FdbErr, FDB_BYTE_ERASED};
use flashdb::{
    blob_make, blob_read, calc_crc32, get_status, read_status, set_status, write_status,
    FlashDevice,
};

/// Minimal in-memory NOR flash for the equivalence test.
/// Initial state all 0xFF; write ANDs (1->0 only); erase resets to 0xFF.
struct EquivFlash {
    data: Vec<u8>,
}

impl EquivFlash {
    fn new(size: usize) -> Self {
        Self {
            data: vec![FDB_BYTE_ERASED; size],
        }
    }
}

impl FlashDevice for EquivFlash {
    fn read(&self, addr: u32, buf: &mut [u8]) -> Result<(), FdbErr> {
        let addr = addr as usize;
        let end = addr.checked_add(buf.len()).ok_or(FdbErr::ReadErr)?;
        if end > self.data.len() {
            return Err(FdbErr::ReadErr);
        }
        buf.copy_from_slice(&self.data[addr..end]);
        Ok(())
    }
    fn write(&mut self, addr: u32, buf: &[u8]) -> Result<(), FdbErr> {
        let addr = addr as usize;
        let end = addr.checked_add(buf.len()).ok_or(FdbErr::WriteErr)?;
        if end > self.data.len() {
            return Err(FdbErr::WriteErr);
        }
        for (i, &b) in buf.iter().enumerate() {
            self.data[addr + i] &= b; // NOR: can only change 1->0
        }
        Ok(())
    }
    fn erase(&mut self, addr: u32, size: u32) -> Result<(), FdbErr> {
        let addr = addr as usize;
        let end = addr.checked_add(size as usize).ok_or(FdbErr::EraseErr)?;
        if end > self.data.len() {
            return Err(FdbErr::EraseErr);
        }
        self.data[addr..end].fill(FDB_BYTE_ERASED);
        Ok(())
    }
    fn len(&self) -> usize {
        self.data.len()
    }
}

// ---------------------------------------------------------------------------
// CRC32 — golden values from the C table (fdb_utils.c:21-89)
// ---------------------------------------------------------------------------

#[test]
fn c_equiv_crc32_empty() {
    // c: fdb_calc_crc32(0, "", 0) == 0
    assert_eq!(calc_crc32(0, &[]), 0x0000_0000);
}

#[test]
fn c_equiv_crc32_canonical_check_value() {
    // The canonical CRC-32/ISO-HDLC check value for "123456789" is 0xCBF43926.
    // FlashDB's fdb_calc_crc32 uses exactly this algorithm (init 0xFFFFFFFF,
    // poly 0xEDB88320, final XOR 0xFFFFFFFF), so the C and Rust outputs match.
    assert_eq!(calc_crc32(0, b"123456789"), 0xCBF4_3926);
}

#[test]
fn c_equiv_crc32_known_strings() {
    // Verify determinism and that accumulation across slices is equivalent to a
    // single shot (a property the C API relies on when streaming CRC updates).
    let whole = calc_crc32(0, b"FlashDB");
    let mut acc = 0u32;
    acc = calc_crc32(acc, b"Flash");
    acc = calc_crc32(acc, b"DB");
    assert_eq!(acc, whole);
    // Non-empty input must differ from the empty-input CRC.
    assert_ne!(calc_crc32(0, b"FlashDB"), 0);
}

// ---------------------------------------------------------------------------
// Status table — golden encodings from the C comment (fdb_utils.c:91-107)
// ---------------------------------------------------------------------------
//
// For FDB_WRITE_GRAN == 1, status_num == 4 the C comment lists:
//   status0 -> 0xFF, status1 -> 0x7F, status2 -> 0x3F, status3 -> 0x1F

#[test]
fn c_equiv_set_status_gran1_encoding_table() {
    let mut table = [0u8; 2];

    // index 0 -> all erased, byte_index == usize::MAX, table stays 0xFF
    let bi = set_status(&mut table, 4, 0);
    assert_eq!(bi, usize::MAX);
    assert_eq!(table[0], 0xFF);

    set_status(&mut table, 4, 1);
    assert_eq!(table[0], 0x7F, "C table: status1 == 0x7F");

    set_status(&mut table, 4, 2);
    assert_eq!(table[0], 0x3F, "C table: status2 == 0x3F");

    set_status(&mut table, 4, 3);
    assert_eq!(table[0], 0x1F, "C table: status3 == 0x1F");
}

#[test]
fn c_equiv_get_status_round_trip() {
    // Each encoded index must decode back to itself, matching C _fdb_get_status.
    let mut table = [0u8; 2];
    for idx in 0..4u32 {
        set_status(&mut table, 4, idx as usize);
        assert_eq!(get_status(&table, 4), idx as usize);
    }
}

#[test]
fn c_equiv_write_read_status_on_flash() {
    // Persisting a status via write_status and reading it back via read_status
    // must round-trip on a real (simulated) flash, exactly as the C code does.
    let mut flash = EquivFlash::new(4096);
    let mut table = [0u8; 2];
    for idx in 0..4u32 {
        flash.erase(0, 4096).unwrap();
        write_status(&mut flash, 0, &mut table, 4, idx as usize).unwrap();
        assert_eq!(
            read_status(&flash, 0, &mut table, 4),
            idx as usize,
            "persisted status must round-trip through flash"
        );
    }
}

// ---------------------------------------------------------------------------
// Blob API — round-trip equivalence (fdb_utils.c:221-249)
// ---------------------------------------------------------------------------

#[test]
fn c_equiv_blob_round_trip() {
    let mut flash = EquivFlash::new(4096);
    let payload: Vec<u8> = (0..32u8).collect();
    flash.write(0, &payload).unwrap();

    let mut buf = vec![0u8; 32];
    let mut blob = blob_make(&mut buf);
    blob.saved_addr = 0;
    blob.saved_len = 32;

    let read_len = blob_read(&flash, &mut blob);
    assert_eq!(read_len, 32, "blob_read must return the full saved length");
    assert_eq!(&buf[..32], &payload[..], "blob round-trip data must match");
}

#[test]
fn c_equiv_blob_truncates_to_saved_len() {
    // c: if (read_len > blob->saved.len) read_len = blob->saved.len;
    let mut flash = EquivFlash::new(4096);
    flash.write(0, &[0xAA; 16]).unwrap();

    let mut buf = vec![0u8; 32];
    let mut blob = blob_make(&mut buf);
    blob.saved_addr = 0;
    blob.saved_len = 16;

    let read_len = blob_read(&flash, &mut blob);
    assert_eq!(read_len, 16, "blob_read truncated to saved_len");
    assert_eq!(&buf[..16], &[0xAA; 16]);
}

#[test]
fn c_equiv_blob_make_sets_defaults() {
    // blob_make must size the blob to the buffer and reset saved.* to unused.
    let mut buf = vec![0u8; 8];
    let blob: FdbBlob = blob_make(&mut buf);
    assert_eq!(blob.size, 8);
    assert_eq!(blob.saved_len, 0);
}
