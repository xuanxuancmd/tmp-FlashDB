# 类型双关（Type Punning）→ Rust 安全转换

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 代码中大量使用指针强转进行类型双关，典型场景是 flash I/O：

```c
// C: 把 struct 当 uint32_t[] 传给 flash 读写
_fdb_flash_read(db, addr, (uint32_t *)&sec_hdr, sizeof(struct sector_hdr_data));
_fdb_flash_read(db, addr, (uint32_t *) buf, sizeof(buf));  // buf 是 uint8_t[32]
```

这在 C 中是**未定义行为**（违反严格别名规则），但大多数编译器容忍。Rust 严格别名检查会在编译期拒绝。

## 解决方案

### 方案 A：`zerocopy` crate（推荐，零拷贝）

```rust
use zerocopy::{AsBytes, FromBytes, FromZeroes};

#[repr(C)]
#[derive(AsBytes, FromBytes, FromZeroes, Clone, Copy)]
struct SectorHdrData {
    status_table: [u8; 4],
    magic: u32,
    combined: u32,
    reserved: u32,
}

// flash 读取：直接读入 struct
fn read_sector_hdr(flash: &impl FlashRead, addr: u32) -> Result<SectorHdrData, FlashErr> {
    let mut buf = [0u8; core::mem::size_of::<SectorHdrData>()];
    flash.read(addr, &mut buf)?;
    Ok(SectorHdrData::read_from_bytes(&buf).unwrap())
}

// flash 写入：直接从 struct 写出
fn write_sector_hdr(flash: &mut impl FlashWrite, addr: u32, hdr: &SectorHdrData) -> Result<(), FlashErr> {
    flash.write(addr, hdr.as_bytes())?;
    Ok(())
}
```

### 方案 B：`bytemuck` crate（替代，更宽松）

```rust
use bytemuck::{Pod, Zeroable, cast, cast_mut};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SectorHdrData {
    status_table: [u8; 4],
    magic: u32,
    combined: u32,
    reserved: u32,
}

// 整体转换
let hdr: SectorHdrData = cast(buf);  // &[u8] → &SectorHdrData
let bytes: &[u8] = cast(&hdr);       // &SectorHdrData → &[u8]
```

### 方案 C：手动 `from_le_bytes`（最安全，无 unsafe）

```rust
#[repr(C)]
struct SectorHdrData {
    magic: u32,
    combined: u32,
}

impl SectorHdrData {
    fn from_bytes(buf: &[u8]) -> Self {
        assert!(buf.len() >= 8);
        Self {
            magic: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            combined: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        }
    }
    fn to_bytes(&self) -> [u8; 8] {
        let mut buf = [0u8; 8];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..8].copy_from_slice(&self.combined.to_le_bytes());
        buf
    }
}
```

## Flash I/O API 设计

C 的 flash API 接受 `void *buf`，Rust 应改为接受 `&[u8]` / `&mut [u8]`：

```rust
// C: fdb_err_t _fdb_flash_read(fdb_db_t db, uint32_t addr, void *buf, size_t size);
// Rust:
pub trait FlashStorage {
    fn read(&self, addr: u32, buf: &mut [u8]) -> Result<(), FlashError>;
    fn write(&mut self, addr: u32, buf: &[u8]) -> Result<(), FlashError>;
    fn erase(&mut self, addr: u32, size: u32) -> Result<(), FlashError>;
}
```

所有调用点从 `(uint32_t *)&struct` 改为 `struct.as_bytes()`。

## ❌ 禁止模式

```rust
// 禁止：transmute 类型双关
let hdr: SectorHdrData = unsafe { core::mem::transmute(buf) };

// 禁止：裸指针 reinterpret
let hdr = unsafe { &*(buf.as_ptr() as *const SectorHdrData) };

// 禁止：core::mem::zeroed() 初始化非 Copy 类型
let hdr: SectorHdrData = unsafe { core::mem::zeroed() };
```

## 注意事项

- `#[repr(C)]` struct 可能有 padding 字段，`zerocopy::AsBytes` 要求无 padding 或 padding 已填充
- 条件 padding（如 `FDB_WRITE_GRAN == 64` 时添加 `padding[4]`）需在 Rust 中用 `#[cfg]` 或 const generic 精确复现
- 对齐要求：`#[repr(align(N))]` 用于满足 flash 写粒度对齐
