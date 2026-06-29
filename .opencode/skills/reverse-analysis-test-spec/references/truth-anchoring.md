# Truth Anchoring 真值锚定

> 返回 [SKILL.md](../SKILL.md) — Phase 6 可选

**不可删理由**:LLM 不会主动思考"金标准本身可能错"。本文件提供"哪些断言需要真实运行结果"的判断标准,以及人工补充流程。

---

## 何时启用 Phase 6

当某个 Claim (Contract 条款、Property 声明、或 Witness 断言)**无法从源码直接读出具体值**,必须来自参考实现真实运行时启用。

| 需要人工补充真实运行结果 | 可直接从源码读出 |
|---------------------|----------------|
| 序列化后的字节序列 | 枚举 discriminant 值 |
| CRC / SHA / 哈希计算结果 | magic number 常量 |
| 压缩 / 加密 / 编码等非平凡算法的输出 | struct sizeof / offsetof |
| 性能边界(处理 N 条记录耗时) | 错误码定义 |
| 并发执行的真实行为 | 状态枚举成员列表 |
| 跨 Contract 集成行为的真实输出(如多个契约涉及的完整写入流程产出的最终磁盘状态) | — |

## 流程(4 步)

### 1. Skill 生成 `pending_human_input.md`

对每个无法直接填写具体值的断言,skill 在 `spec/pending_human_input.md` 中写一个条目:

```markdown
## CONTRACT-{ID} / PROPERTY-{ID} / WITNESS-{ID}: {标题}
- 源码位置: {file}:{line}
- 断言描述: {这个断言要验证什么}
- 期望类型: {u32 / 字节序列 / 字符串 / struct 字段值 / ...}
- 填入建议: {获取这个值的具体方法}
- 填入: ___
```

同时在 `contracts.md` / `properties.md` / `witnesses.md` 中对应条款写描述性占位文本:

```markdown
**Postcondition** 输出字节序列 [待补充: 运行参考实现,dump 出完整字节序列]
```

### 2. 人工获取真实运行结果

按 `填入建议` 列出的方法,在隔离环境运行参考实现:

| 锚定源 | 适用 | 优先级 |
|--------|------|:---:|
| **标准测试向量**(RFC / NIST 公开文件) | CRC、SHA、base64、gzip 等标准算法 | **最高** |
| 标准库函数(`crc32fast` / `sha2` 等) | 标准算法等价验证 | 高 |
| 参考实现真实运行(隔离环境) | 项目专用算法、业务逻辑 | 中 |
| 多实现交叉验证(≥ 2 个独立实现) | 高价值断言(加密、协议) | 中 |

**优先用已有权威源**(如 NIST 测试向量) — 比自己搭环境跑更可靠、更可重现。

隔离环境选择(Ubuntu Linux 为例):
- **WSL 2**:`wsl --install` 后 `apt install build-essential`
- **Docker**:`docker run --rm -v ./:/src gcc:latest bash`
- **CI runner**:GitHub Actions / GitLab CI
- Python/Java/Go/TS 项目通常 Windows 有原生运行时,可直接本地运行

### 3. 人工回填 pending 清单

把真实运行结果填入 `pending_human_input.md` 每个条目的"填入: ___"行:

```markdown
## CONTRACT-07: CRC32 对空输入的输出
- 源码位置: utils.c:77
- 断言描述: CRC32 算法对 0 字节输入计算的输出值
- 期望类型: u32 (4 字节十六进制)
- 填入建议: 运行 `calc_crc32(ptr::null(), 0)` 或查 IEEE 802.3 测试向量
- 填入: **0x00000000**(IEEE 802.3 标准向量)
```

### 4. 人工回填 spec

把 `pending_human_input.md` 中每个条目的"填入"值,复制回 `contracts.md` / `properties.md` / `witnesses.md` 对应的条款或断言行(替换描述性占位文本):

```markdown
**Postcondition** 输出字节序列为 [0x00, 0x00, 0x00, 0x00]  (CRC32 of empty input, IEEE 802.3)
```

全部回填完成后:
1. 删除 `pending_human_input.md`(表示 Phase 6 完成)
2. 标记 spec 文件为冻结(只读)
3. 跑覆盖度门禁(指标 G:检查 spec 中是否还有"待补充"占位文本未填)

## 反模式

| ❌ 反模式 | ✅ 正确做法 |
|---------|-----------|
| AI 凭空推测具体值 | 产出 `pending_human_input.md`,由人工补充 |
| 在 spec 中写猜测值(如"大概 0x1234") | 写描述性占位,列入 pending 清单 |
| 只跑 1 个输入就回填(单点可能错) | 跑 ≥ 3 个不同输入交叉验证 |
| 在主项目环境运行参考实现(耦合) | 在隔离环境运行 |
| 把参考实现行为当绝对真理 | 同时验证参考实现与标准/规范一致 |
| AI 直接改 spec 文件填写真实值 | AI 只写描述性占位;真实值由人工填入 |

## 标准算法推荐锚定源

| 算法 | 推荐锚定源 | 示例 |
|------|----------|------|
| CRC32 | IEEE 802.3 测试向量 | `"123456789"` → `0xCBF43926` |
| SHA-256 | NIST CAVP 测试向量 | (公开文件) |
| gzip / deflate | zlib + 公开测试向量 | (公开文件) |
| base64 | RFC 4648 附录测试用例 | (RFC 文本) |
| URL 编码 | WHATWG URL 标准测试 | (WHATWG 网站) |

**优先使用已有测试向量** — 比自己运行环境更权威、更稳定、更易重现。

## 锚定正确性的自检

填完真实值后,人工自检:

- ✅ 值与标准测试向量一致(如 CRC32 空输入必为 `0x00000000`)
- ✅ ≥ 3 个不同输入覆盖边界(不能只跑 1 个就认为对)
- ✅ 值的类型与断言描述一致(u32 / 字节序列 / 字符串)
- ✅ 值看起来合理(如 CRC 不为全零、字节序符合预期)

**黄金规则**:真实运行结果与标准测试向量不一致 → **不是运行环境错就是参考实现错**。停下来调查,不要盲目回填 spec。

## 产出

Phase 6 完成后产出:

| 文件 | 内容 |
|------|------|
| `spec/contracts.md` / `properties.md` / `witnesses.md` | 所有"待补充"占位文本已替换为真实值 |
| `spec/golden_fixtures.json`(可选) | 多场景的 (input, expected_output) 对,便于后续 diff testing |
| `coverage_report.md` | 追加 `true_anchored` 列:每个 Claim 是否已锚定 |

`pending_human_input.md` 在 Phase 6 完成后被**删除** — 它的使命是清单,不是长期资产。
