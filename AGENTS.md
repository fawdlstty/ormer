# 项目结构

- `Cargo.toml`：Cargo workspace 配置。
- `ormer/`：ORM 主库。
  - `src/lib.rs`：公共 API 与模块导出。
  - `src/model.rs`：模型、字段、值与建表 SQL。
  - `src/query/`：查询构建器、过滤器与 SQL 生成。
  - `src/abstract_layer/`：数据库抽象层、连接池、事务与各数据库后端。
  - `src/hooks.rs`：生命周期钩子。
  - `src/migration.rs`：数据库迁移。
  - `src/raw_sql.rs`：原生 SQL。
  - `tests/`：集成测试及测试辅助代码。
- `ormer-derive/`：`Model` 与 `ModelEnum` 派生过程宏。
- `README.md`、`README.zh.md`：中英文项目说明。
- `docs/guide/`、`docs/en/guide/`：中英文使用文档。
- `docs/.vuepress/`、`docs/package.json`：文档站点配置与构建脚本。

# 编译与测试

- 修改代码后，必须同时进行编译和测试，不能只执行 `cargo check`。
- 根据改动涉及的数据库后端，使用对应的 feature 组合执行：

```bash
cargo build --workspace --no-default-features --features <features>
cargo test --workspace --no-default-features --features <features>
```

- 至少验证默认的 `sqlite`，以及改动涉及的 `postgresql`、`mysql`、`mssql` 或组合；需要覆盖全部后端时使用 `full` 或 `--all-features`。
- PostgreSQL、MySQL 集成测试需要可用的测试数据库，并可通过 `ORMER_TEST_POSTGRES`、`ORMER_TEST_MYSQL` 配置连接地址。

# 文档同步

- 新增功能后，必须同步更新 `docs/guide/` 与 `docs/en/guide/` 中对应的中英文文档，必要时同步更新 `README.md` 与 `README.zh.md`。
- 只用最简洁的文字和示例说明功能及用法，不保留多余内容。
