# offsetof via null 解引用 → `offset_of!`

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 代码用 null 指针解引用计算字段偏移量，定义 on-flash 二进制协议：

```c
// C: 用 null 指针 trick 计算 offsetof
#define SECTOR_STORE_OFFSET  ((unsigned long)(&((struct sector_hdr_data *)0)->status_table.store))
#define KV_MAGIC_OFFSET      ((unsigned long)(&((struct kv_hdr_data *)0)->magic))
#define SECTOR_END0_TIME_OFFSET ((unsigned long)(&((struct sector_hdr_data *)0)->end_info[0].time))
```

这在 C 中是**未定义行为**（null 指针解引用），但大多数编译器产生正确结果。Rust 中不允许这样做。

## 解决方案

### 方案 A：`core::mem::offset_of!`（Rust 1.77+，推荐）

```rust
// Rust 1.77+ 稳定的 offset_of 宏
const SECTOR_STORE_OFFSET: usize = core::mem::offset_of!(SectorHdrData, status_table.store);
const KV_MAGIC_OFFSET: usize = core::mem::offset_of!(KvHdrData, magic);
const SECTOR_END0_TIME_OFFSET: usize = core::mem::offset_of!(SectorHdrData, end_info[0].time);
```

### 方案 B：`memoffset` crate（Rust < 1.77）

```rust
// Cargo.toml: memoffset = "0.9"
use memoffset::offset_of;

const SECTOR_STORE_OFFSET: usize = offset_of!(SectorHdrData, status_table.store);
```

### 方案 C：手动常量计算（无依赖）

```rust
// 对于简单布局，手动计算偏移
const SECTOR_STORE_OFFSET: usize = 0;  // status_table.store 是第一个字段
const SECTOR_DIRTY_OFFSET: usize = 2;  // status_table.dirty 在 store 之后
const SECTOR_MAGIC_OFFSET: usize = 4;  // magic 在 status_table(4字节) 之后
```

**注意**：方案 C 容易出错，仅在无依赖且布局简单时使用，必须用 `assert!` 验证。

## 完整示例

```rust
#[repr(C)]
struct SectorHdrData {
    status_table: StatusTable,  // 嵌套结构体
    magic: u32,
    combined: u32,
    reserved: u32,
}

#[repr(C)]
struct StatusTable {
    store: [u8; 2],
    dirty: [u8; 2],
}

// 用 offset_of! 替代 C 宏
const SECTOR_STORE_OFFSET: usize = core::mem::offset_of!(SectorHdrData, status_table.store);
const SECTOR_MAGIC_OFFSET: usize = core::mem::offset_of!(SectorHdrData, magic);

// 验证偏移与 C 版本一致（编译时断言）
const _: () = assert!(SECTOR_STORE_OFFSET == 0);
const _: () = assert!(SECTOR_MAGIC_OFFSET == 4);

// 使用：在 flash 特定偏移写入字段
fn write_magic(flash: &mut impl FlashWrite, base_addr: u32, magic: u32) -> Result<(), FlashErr> {
    flash.write(base_addr + SECTOR_MAGIC_OFFSET as u32, &magic.to_le_bytes())?;
    Ok(())
}
```

## 验证策略

翻译后**必须**用编译时断言验证所有偏移：

```rust
// 编译时验证 —— 如果偏移不匹配，编译失败
const _: () = {
    assert!(core::mem::offset_of!(SectorHdrData, status_table.store) == 0);
    assert!(core::mem::offset_of!(SectorHdrData, status_table.dirty) == 2);
    assert!(core::mem::offset_of!(SectorHdrData, magic) == 4);
    assert!(core::mem::offset_of!(SectorHdrData, combined) == 8);
};
```

## ❌ 禁止模式

```rust
// 禁止：null 指针解引用 trick（Rust 中是 UB）
const OFFSET: usize = unsafe { (&(*(0 as *const SectorHdrData)).magic) as *const _ as usize };

// 禁止：transmute 空指针
const OFFSET: usize = {
    let ptr = 0 as *const SectorHdrData;
    unsafe { (&(*ptr).magic as *const _) as usize }
};
```
