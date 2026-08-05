# 钩子系统 (Hooks)

Hooks 为写操作提供可失败的生命周期回调。所有回调都接收 `HookContext` 并返回 `ormer::Result<()>`；业务校验应返回错误，而不是 panic。

| Hook Trait | 时机 | 签名 |
| --- | --- | --- |
| `BeforeInsert` | SQL 插入前 | `async fn before_insert(&mut self, ctx: &mut HookContext<'_>) -> Result<()>` |
| `AfterInsert` | SQL 插入成功后 | `async fn after_insert(&self, ctx: &mut HookContext<'_>) -> Result<()>` |
| `BeforeUpdate` | SQL 更新前 | `async fn before_update(&mut self, ctx: &mut HookContext<'_>) -> Result<()>` |
| `AfterUpdate` | SQL 更新影响至少一行后 | `async fn after_update(&self, ctx: &mut HookContext<'_>) -> Result<()>` |
| `BeforeDelete` | SQL 删除前 | `async fn before_delete(&self, ctx: &mut HookContext<'_>) -> Result<()>` |
| `AfterDelete` | SQL 删除影响至少一行后 | `async fn after_delete(&self, ctx: &mut HookContext<'_>) -> Result<()>` |

## 插入

插入 Hook 使用可变模型输入，以便 `BeforeInsert` 可以规范化或补充字段。实现 `BeforeInsert` 和 `AfterInsert` 后，`insert`、`insert_or_update`、`insert_or_ignore` 以及对应事务执行器都会按 `BeforeInsert -> SQL -> AfterInsert` 调用。

```rust
use ormer::{
    AfterInsert, BeforeInsert, Database, DbType, HookContext, Model,
};

#[derive(Debug, Model)]
#[table = "users"]
struct User {
    #[primary(auto)]
    id: i32,
    email: String,
}

#[async_trait::async_trait]
impl BeforeInsert for User {
    async fn before_insert(&mut self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        self.email = self.email.trim().to_lowercase();
        if !self.email.contains('@') {
            return Err(anyhow::anyhow!("invalid email"));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl AfterInsert for User {
    async fn after_insert(&self, _ctx: &mut HookContext<'_>) -> ormer::Result<()> {
        Ok(())
    }
}

let db = Database::connect(DbType::Sqlite, "app.db").await?;
db.create_table::<User>().execute().await?;

let mut user = User {
    id: 0,
    email: "  Alice@example.com ".into(),
};
user.id = db.insert(&mut user).execute().await?;
```

批量插入传入可变集合。框架会逐条调用 Hook，`ctx.batch_index()` 是当前记录在集合中的索引。

```rust
let mut users = vec![user_a, user_b];
db.insert(&mut users).execute().await?;
```

`BeforeInsert` 返回错误时不会执行 SQL。`AfterInsert` 返回错误时 SQL 已执行；普通数据库操作已无法自动撤销，事务操作应由调用方随后执行 `rollback()`。

## 更新和删除

更新和删除没有模型输入时无法确定 Hook 的作用对象，因此使用 `execute_with_hooks` 显式提供模型。批量场景使用 `execute_models_with_hooks`，HookContext 会带上批量索引。

```rust
db.update::<User>()
    .set_model(&user)
    .execute_with_hooks(&mut user)
    .await?;

db.delete::<User>()
    .filter(|fields| fields.id.eq(user.id))
    .execute_with_hooks(&user)
    .await?;
```

在事务中执行插入时，`ctx.in_transaction()` 为 `true`。Hook 错误会作为 `Result` 返回，不会自动提交事务；调用方应根据业务语义选择 `commit()` 或 `rollback()`。

## HookContext

`HookContext` 提供：

- `operation()`：当前操作类型，取值为 `Insert`、`Update` 或 `Delete`。
- `batch_index()`：批量操作中的记录索引；单条操作为 `None`。
- `in_transaction()`：当前操作是否由 `Transaction` 执行。
