# P0/P1 Structural Fixes Summary

## 修复的问题

### P0 级（必须修复 - 已完成）

#### P0-1: FieldKind::from_type - nested attr 作用到叶子，容器保留

**问题**:
- `#[carve(nested)] Option<Profile>` 被整体当成 `Nested`，而不是 `Option(Nested)`
- 导致 `inner.is_nested()` 分支永远不触发

**修复**:
- `FieldKind::from_type` 现在将 nested attr 传播到叶子类型
- 容器结构（Option/Vec/Map）被保留
- `Option<Profile>` with `nested=true` → `Option(Nested)`
- `Vec<Profile>` with `nested=true` → `Vec(Nested)`
- `HashMap<String, Profile>` with `nested=true` → `Map{value: Nested}`

**文件**: `crates/carve-state-derive/src/field_kind.rs`

**测试覆盖**:
- `test_option_nested_reader` - Option<Nested> 读取
- `test_option_nested_none` - Option<Nested> null 处理
- `test_vec_nested_writer` - Vec<Nested> 写入
- `test_vec_nested_accessor` - Vec<Nested> accessor
- `test_map_nested_value_accessor` - Map<String, Nested> accessor

---

#### P0-2: Nested 类型引用使用 trait 关联类型（跨模块支持）

**问题**:
- 通过字符串拼接生成 `ProfileWriter` 等类型名
- 跨模块时找不到类型（`crate::models::Profile` → `ProfileWriter` 找不到）
- 泛型/别名/路径都会失败

**修复**:
- Reader: `<#field_ty as ::carve_state::CarveViewModel>::Reader<'a>`
- Writer: `<#field_ty as ::carve_state::CarveViewModel>::Writer`
- Accessor: `<#field_ty as ::carve_state::CarveViewModel>::Accessor<'a>`
- 使用 trait 调用而不是字符串拼接

**文件**:
- `crates/carve-state-derive/src/codegen/reader.rs`
- `crates/carve-state-derive/src/codegen/writer.rs`
- `crates/carve-state-derive/src/codegen/accessor.rs`

**测试覆盖**:
- `test_cross_module_nested_reader` - 跨模块 nested 读取
- `test_cross_module_nested_writer` - 跨模块 nested 写入

---

#### P0-3: Guard 命名包含字段名（避免重复类型冲突）

**问题**:
- Guard 名只基于类型名 `ProfileWriterGuard`
- 同一 struct 中两个 `Profile` 字段会生成重复定义

**修复**:
- Guard 名包含字段名：`{Field}WriterGuard`、`{Field}AccessorGuard`
- 使用 PascalCase 转换：`leader` → `LeaderWriterGuard`
- 每个字段有独立的 Guard 类型

**文件**:
- `crates/carve-state-derive/src/codegen/writer.rs` (added `to_pascal_case`)
- `crates/carve-state-derive/src/codegen/accessor.rs` (added `to_pascal_case`)

**测试覆盖**:
- `test_duplicate_nested_type_writer` - 同类型多字段 writer
- `test_duplicate_nested_type_accessor` - 同类型多字段 accessor
- `test_guards_are_field_scoped` - 编译时验证 guard 独立

---

### P1 级（语义/一致性 - 已完成）

#### P1-6: Accessor guard Drop 使用 drain 而不是 clone

**问题**:
- Guard Drop 中 `ops.iter().cloned()` 复制所有操作
- ops 没被清空（虽然只 drop 一次，但语义不清晰）

**修复**:
- 使用 `ops.drain(..)` 移动 ops 而不是复制

**文件**: `crates/carve-state-derive/src/codegen/accessor.rs`

---

#### P1-7: flatten 属性产生 compile error

**问题**:
- `#[carve(flatten)]` 被解析但不实现
- 用户会误以为支持，实际被静默忽略

**修复**:
- 在 codegen 时检测 `flatten=true` 并产生编译错误
- 清晰的错误信息指导用户使用替代方案

**文件**: `crates/carve-state-derive/src/codegen/mod.rs`

**测试覆盖**:
- `compile_fail_flatten.rs` - 验证编译错误

---

#### P1-4: from_value 错误包含 path 信息

**问题**:
- `from_value()` 错误不包含字段路径
- 嵌套结构错误难以定位
- 用户无法知道具体哪个字段出错

**修复**:
- 缺失字段返回 `PathNotFound { path }` 错误
- 类型不匹配返回 `TypeMismatch { path, expected, found }` 错误
- 所有错误包含完整的字段路径（JSON Pointer 格式 `$.field`）

**文件**: `crates/carve-state-derive/src/codegen/view_model.rs`

**测试覆盖**:
- `test_from_value_missing_field_error_has_path` - 缺失字段有路径
- `test_from_value_type_mismatch_error_has_path` - 类型错误有路径
- `test_from_value_null_for_required_field` - null 作为必需字段
- `test_nested_from_value_error_has_full_path` - 嵌套错误完整路径
- `test_error_display_includes_path` - 错误显示包含路径
- `test_type_mismatch_error_display` - 类型错误显示

---

#### P1-5: Reader Option 语义区分 missing/null/present

**问题**:
- Reader 中 Option 字段未明确区分缺失和 null
- 语义不清晰，用户难以理解行为

**修复**:
- 字段缺失 (missing) → `Ok(None)`
- 字段为 null → `Ok(None)`
- 字段存在且非 null → 反序列化值
- 类型错误 → `Err(TypeMismatch { path, expected, found })`
- 明确的文档注释说明三种情况

**文件**: `crates/carve-state-derive/src/codegen/reader.rs`

**测试覆盖**:
- `test_reader_option_field_missing` - 缺失字段返回 None
- `test_reader_option_field_null` - null 字段返回 None
- `test_reader_option_field_present` - 存在字段返回 Some
- `test_reader_option_field_type_mismatch` - 类型错误有路径
- `test_reader_required_field_missing` - 必需字段缺失错误
- `test_reader_required_field_type_mismatch` - 必需字段类型错误

---

## 测试结果

### 新增测试

**P0 回归测试** (`tests/p0_regression_tests.rs`):
- 10 个测试覆盖所有 P0 修复
- 验证容器 nested、跨模块 nested、重复类型

**不可变性测试** (`tests/immutability_tests.rs`):
- 13 个测试验证确定性状态转换
- 验证 apply_patch 不可变性
- 验证 Accessor 不修改文档
- 验证历史重放和分叉

**P1 错误消息测试** (`tests/p1_error_message_tests.rs`):
- 12 个测试覆盖 P1-4/P1-5 修复
- 验证 from_value 错误包含路径
- 验证 Reader Option 语义
- 验证嵌套类型错误传播

### 测试通过统计

```
单元测试:       61 passed
Accessor:       18 passed
高级场景:       31 passed
Derive 集成:    33 passed
边界情况:       37 passed
不可变性:       13 passed
P0 回归:        10 passed
P1 错误消息:    12 passed
-------------------------
总计:          215 passed
```

**编译失败测试**: flatten 正确产生编译错误 ✅

---

## 架构改进

### 1. 去除字符串拼接类型推断

**之前**:
```rust
let nested_writer = format_ident!("{}Writer", get_type_name(field_ty));
```

**现在**:
```rust
<#field_ty as ::carve_state::CarveViewModel>::Writer
```

**好处**:
- ✅ 跨模块支持
- ✅ 泛型支持
- ✅ 类型别名支持
- ✅ 路径限定支持
- ✅ 更短、更稳定的代码

---

### 2. Guard 作用域隔离

**之前**: 全局共享 guard 名（按类型）
```rust
ProfileWriterGuard  // 所有 Profile 字段共用
```

**现在**: 字段级 guard 名
```rust
LeaderWriterGuard    // leader: Profile
AssistantWriterGuard // assistant: Profile
```

**好处**:
- ✅ 无命名冲突
- ✅ 更清晰的语义
- ✅ 更好的类型安全

---

### 3. 容器 nested 语义修正

**之前**: `#[carve(nested)]` 整体标记
```rust
#[carve(nested)]
profile: Option<Profile>  // → FieldKind::Nested (错误！)
```

**现在**: nested 标记叶子类型
```rust
#[carve(nested)]
profile: Option<Profile>  // → FieldKind::Option(Nested) ✅
```

**好处**:
- ✅ `Option<Nested>` 正确工作
- ✅ `Vec<Nested>` 正确工作
- ✅ `Map<K, Nested>` 正确工作
- ✅ 所有容器 nested 分支触发

---

## 向后兼容性

✅ **完全向后兼容**
- Reader/Writer API 保持不变
- Accessor 作为新增功能
- 现有代码无需修改
- 所有 215 个测试通过（包括原有 193 个 + 新增 22 个）

---

## 关键代码变更

### 修改的文件

| 文件 | 变更 | 说明 |
|------|------|------|
| `field_kind.rs` | 重构 `from_type` | P0-1: nested 传播到叶子 |
| `reader.rs` | Nested 分支 + Option 语义 | P0-2: trait 关联类型 + P1-5: 区分 missing/null |
| `writer.rs` | Nested 分支 + Guard | P0-2/P0-3: trait + PascalCase |
| `accessor.rs` | Nested 分支 + Guard | P0-2/P0-3 + P1-6: drain |
| `view_model.rs` | from_value 错误处理 | P1-4: 包含 path 信息 |
| `codegen/mod.rs` | flatten 检查 | P1-7: compile_error |

### 新增函数

```rust
fn to_pascal_case(s: &str) -> String  // writer.rs, accessor.rs
```

---

## 验证清单

- [x] P0-1: 容器 nested 正确解析 (Option/Vec/Map)
- [x] P0-2: 跨模块 nested 编译通过
- [x] P0-3: 重复 nested 类型编译通过
- [x] P1-4: from_value 错误包含 path 信息
- [x] P1-5: Reader Option 语义区分 missing/null/present
- [x] P1-6: Guard Drop 使用 drain
- [x] P1-7: flatten 产生编译错误
- [x] 所有原有测试通过 (193)
- [x] P0 回归测试通过 (10)
- [x] 不可变性验证通过 (13)
- [x] P1 错误消息测试通过 (12)
- [x] 编译警告清理完成
- [x] 总计 215 个测试全部通过
