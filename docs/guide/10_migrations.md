# 数据库迁移

Ormer 提供两种迁移方式：根据模型生成可检查的增量计划，以及通过 `Migration` trait 编写带版本和校验和的迁移。

## 根据模型生成计划

`migrate_table::<T>()` 会生成可执行的 schema 迁移计划。不存在的表会生成建表步骤，不存在的非主键列会生成 `ADD COLUMN`；已存在列的类型或可空性变化会尽量生成真实迁移，SQLite 需要时会通过重建表完成回填。无法安全推断的数据转换会直接返回错误。

```rust
let plan = db.migrate_table::<User>().plan().await?;

println!("{}", plan.to_sql()?);
for warning in plan.warnings() {
    eprintln!("{warning}");
}

db.migrate_table::<User>().execute().await?;
```

如果目标转换无法安全证明，`plan()` 或 `execute()` 会返回错误。SQLite 的复杂变更通过重建表完成，非法旧数据会在迁移阶段失败并回滚。

## 版本化迁移

实现 `Migration`，并将迁移按版本交给 `db.migrations()`：

```rust
use ormer::{Migration, MigrationRunner, MigrationStep};

struct AddUserEmail;

impl Migration for AddUserEmail {
    fn version(&self) -> u64 {
        1
    }

    fn name(&self) -> &str {
        "add_user_email"
    }

    fn up(&self) -> Vec<MigrationStep> {
        vec![MigrationStep::AddColumn {
            table: "users".into(),
            column: "email".into(),
            definition: "TEXT".into(),
        }]
    }

    fn down(&self) -> Vec<MigrationStep> {
        vec![MigrationStep::Sql {
            sql: "DROP COLUMN email".into(),
        }]
    }
}

let migrations = [AddUserEmail];
let runner: MigrationRunner<'_, AddUserEmail> = db.migrations(&migrations);

let pending = runner.pending().await?;
println!("pending: {}", pending.len());
let applied = runner.execute().await?;
println!("applied: {applied}");
```

如果不需要持有 runner，也可以直接调用数据库入口：

```rust
let pending = db.pending_migrations(&migrations).await?;
let applied = db.apply_migrations(&migrations).await?;
```

已应用的迁移会记录在 `__ormer_migrations` 中。迁移按版本排序，在事务中执行，并保存由名称和 `up()` 内容计算出的 checksum；修改已应用迁移会报错。

可用的 `MigrationStep` 包括 `CreateType`、`AlterType`、`CreateTable`、`AddColumn`、`BackfillColumn`、`AlterColumn`、`AddConstraint`、`CreateIndex`、`AddForeignKey` 和 `Sql`。需要复杂或方言专用 DDL 时使用 `Sql`。

```rust
let history = db.migration_history().await?;
for migration in history {
    println!("{} {} {}", migration.version, migration.name, migration.checksum);
}
```

SQLite 不支持在建表后追加外键；`MigrationStep::AddForeignKey` 在 SQLite 上会返回错误。
