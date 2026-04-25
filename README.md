# ormer

![version](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Ffawdlstty%2Former%2Fmain%2F/ormer/Cargo.toml&query=package.version&label=version)
![status](https://img.shields.io/github/actions/workflow/status/fawdlstty/ormer/rust.yml)

English | [简体中文](README.zh.md)

An ORM framework with a usage style similar to Linq, supporting Turso(SQLite), PostgresQL, MySQL.

# Usage

<!-- [Online Documentation](https://ormer.fawdlstty.com) -->

Add the library reference:

```sh
cargo add ormer --features turso # can be changed to mysql, postgresql
cargo add tokio --features full
```

#### Define Model

Use the `#[derive(Model)]` macro to define your database model:

```rust
use ormer::Model;

#[derive(Model, Debug, Clone)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    #[unique]
    name: String,
    #[index]
    age: i32,
    email: Option<String>,
}
```

#### Access Database

```rust
#[tokio::main]
async fn main() {
    let db = ormer::Database::connect(ormer::DbType::Turso, ":memory:").await?;

    db.create_table::<User>().await?;
    db.insert(&User {
        id: 0, // will be auto-generated
        name: "Alice".to_string(),
        age: 25,
        email: Some("alice@example.com".to_string()),
    }).await?;
    let users = db.select::<User>().collect::<Vec<User>>().await?;
}
```

### Features

- **turso** (default): Use Turso/libsql database backend
- **postgresql**: Use PostgreSQL database backend
- **mysql**: Use MySQL database backend

Enable features in your `Cargo.toml`:

```toml
[dependencies]
ormer = { version = "0.1", features = ["postgresql"] }
```

## Comparison with Other Rust ORMs

> **Note**: Ormer is a relatively new ORM framework focused on providing a clean API and type-safe query experience. The following comparison is based on the current state of each framework to help developers choose the right tool for their project needs.

### Feature Comparison

| Feature | Ormer | SeaORM | Diesel | Toasty |
|---------|-------|--------|--------|--------|
| Async Support | ✅ Native | ✅ Native | ❌ Requires extra config | ✅ Native |
| Compile-time Checking | ✅ Strong typing | ⚠️ Partial | ✅ Strong typing | ✅ Strong typing |
| Multi-Database Support | ✅ 3 databases | ✅ Multiple | ✅ Multiple | ✅ Multiple |
| Migration System | ❌ Pending | ✅ Built-in | ✅ Built-in | ✅ Supported |
| Streaming Queries | ❌ Pending | ✅ Supported | ❌ Not supported | ✅ Supported |
| Complex Conditional Expressions | ❌ Pending | ✅ Supported | ✅ Supported | ✅ Supported |
| JOIN Types | ⚠️ Basic | ✅ Complete | ✅ Complete | ✅ Complete |
| Relationship Loading Strategy | ❌ Pending | ✅ Eager/Lazy | ✅ Supported | ✅ Supported |
| Database Type Extensions | ❌ Pending | ✅ Rich | ✅ Rich | ✅ Rich |
| Batch Operations | ⚠️ Partial | ✅ Complete | ✅ Complete | ✅ Complete |
| Testing Support | ❌ Pending | ✅ Mock Database | ⚠️ Manual | ⚠️ Manual |
| Hooks & Callbacks | ❌ Pending | ✅ Supported | ❌ Not supported | ✅ Supported |
| Soft Delete | ❌ Pending | ✅ Supported | ❌ Not supported | ❌ Not supported |
| Composite Primary Key | ❌ Pending | ✅ Supported | ✅ Supported | ✅ Supported |
| Enum Types | ❌ Pending | ✅ Supported | ✅ Supported | ✅ Supported |

### Developer Experience Comparison

| Dimension | Ormer | SeaORM | Diesel | Toasty |
|-----------|-------|--------|--------|--------|
| **Model Definition** | ✅ Define once | ⚠️ Requires code generation | ❌ Define twice | ⚠️ Requires code generation |
| **Table SQL** | ✅ Auto-generated | ⚠️ Requires migration files | ❌ Manual writing | ⚠️ Requires migration files |
| **Learning Curve** | ✅ Low (1 day) | ⚠️ Medium (3-5 days) | ❌ High (1-2 weeks) | ⚠️ Medium (3-5 days) |
| **API Simplicity** | ✅ Minimal | ⚠️ Moderate | ❌ Complex | ⚠️ Moderate |
| **Code Duplication** | ✅ Very low | ⚠️ Moderate | ❌ High | ⚠️ Moderate |
| **Query Syntax** | ✅ LINQ-style | ⚠️ Chain calls | ❌ DSL nesting | ⚠️ Chain calls |
| **Type Inference** | ✅ Complete | ⚠️ Partial | ✅ Complete | ✅ Complete |
| **IDE Support** | ✅ Excellent | ⚠️ Good | ⚠️ Good | ⚠️ Good |
| **Error Messages** | ✅ Clear at compile-time | ⚠️ More at runtime | ✅ Strict at compile-time | ⚠️ Mixed |
| **Debugging Difficulty** | ✅ Low | ⚠️ Medium | ❌ High | ⚠️ Medium |

### Engineering Capability Comparison

| Dimension | Ormer | SeaORM | Diesel | Toasty |
|-----------|-------|--------|--------|--------|
| **Ecosystem Maturity** | ⭐⭐ Developing | ⭐⭐⭐⭐⭐ Mature | ⭐⭐⭐⭐⭐ Mature | ⭐⭐ Developing |
| **Documentation Quality** | ⭐⭐⭐ Good | ⭐⭐⭐⭐⭐ Comprehensive | ⭐⭐⭐⭐⭐ Comprehensive | ⭐⭐⭐ Good |
| **Community Activity** | ⭐⭐ Growing | ⭐⭐⭐⭐⭐ Active | ⭐⭐⭐⭐⭐ Active | ⭐⭐ Growing |
| **Production Ready** | ⚠️ Simple scenarios | ✅ Complex scenarios | ✅ Complex scenarios | ⚠️ Simple scenarios |
| **Performance** | ⭐⭐⭐⭐ Excellent | ⭐⭐⭐⭐ Excellent | ⭐⭐⭐⭐⭐ Outstanding | ⭐⭐⭐⭐ Excellent |
| **Package Size** | ✅ Lightweight | ⚠️ Heavier | ⚠️ Heavier | ✅ Lightweight |
| **Compilation Speed** | ✅ Fast | ⚠️ Moderate | ❌ Slow (complex macros) | ✅ Fast |

### Framework Characteristics Summary

#### Ormer Advantages ✅
- **Rapid Prototyping**: Define models once, auto-generate table SQL, no extra tools needed
- **Low Learning Cost**: Intuitive LINQ-style API, get started in 1 day
- **Code Simplicity**: No code duplication, no need to maintain multiple definitions or migration files (basic scenarios)
- **Compile-time Safety**: Complete type inference, errors caught at compile-time
- **Lightweight Projects**: Small package size, fast compilation, ideal for microservices and small projects
- **Multi-Database Switching**: Unified API, switch databases without modifying business code
