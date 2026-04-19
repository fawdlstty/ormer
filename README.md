# Ormer - Rust LINQ-like ORM

<div align="center">

**极简语法 + 编译期优化 = 高性能类型安全 ORM**

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org)

</div>

## 特性

- 🚀 **极简语法**：类 LINQ 的链式调用 API
- ⚡ **编译期优化**：过程宏解析，零运行时反射
- 🔒 **类型安全**：字段错误在编译期捕获
- 🎯 **高性能**：接近手写 SQL 的性能
- 🌊 **异步支持**：原生 async/await

## 快速开始

### 定义模型

```rust
use ormer::Model;

#[derive(Model)]
#[table = "users"]
struct User {
    #[primary_key]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}
```

### 查询数据

```rust
// 简单查询
let adults = User::query()
    .filter(|u| u.age.gt(18))
    .all(&db).await?;

// 复杂查询
let users = User::query()
    .filter(|u| u.age.ge(18).and(u.name.like("A%")))
    .order_by(|u| u.name.asc())
    .limit(10)
    .offset(20)
    .all(&db).await?;
```

## 项目结构

```
ormer/
├── ormer/              # 核心库
│   ├── src/
│   │   ├── lib.rs
│   │   ├── model.rs
│   │   └── query/
│   └── tests/
├── ormer-derive/       # 过程宏库
│   └── src/
├── DESIGN.md           # 详细设计文档
└── README.md
```

## 文档

- [设计文档](DESIGN.md) - 完整的技术架构和设计方案
- [API 文档](https://docs.rs/ormer) - 即将上线

## 开发状态

✅ **语法验证通过** - 所有测试用例成功通过

当前已实现：
- ✅ Model trait 和 derive 宏
- ✅ QueryBuilder 链式调用
- ✅ SQL 生成器（PostgreSQL/MySQL/SQLite 兼容）
- ✅ 类型安全的过滤表达式
- ✅ 编译期常量优化

规划中：
- 🚧 完整闭包 AST 解析
- 🚧 数据库适配器集成
- 🚧 INSERT/UPDATE/DELETE
- 🚧 JOIN 关联查询

## 运行测试

```bash
cargo test --workspace
```

## 许可证

MIT License

## 贡献

欢迎提交 Issue 和 Pull Request！
