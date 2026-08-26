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
            return Err(ormer::ormer_error!("invalid email"));
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

更新和删除的普通 `execute()` 默认执行纯 SQL；需要模型级 Hook 时使用 `execute_with_hooks` 显式提供模型。批量场景使用 `execute_models_with_hooks`，HookContext 会带上批量索引。

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

所有写入 Hook 默认开启。仅需跳过当前执行链时使用 `without_hooks()`；它不会影响其他任务或连接。

当前版本没有跨连接的数据库变更监听器。原生 SQL、其他连接或其他进程的写入不会构造模型并触发 `WriteHook`。

## SQL Trace

`sql_trace()` 用于注册全局 SQL 执行回调，可记录 SQL、参数视图、耗时、错误、慢 SQL，并可在执行前改写 SQL 文本。

```rust
let db = ormer::Database::connect(ormer::DbType::Sqlite, ":memory:").await?;

db.sql_trace()
    .before(|sql| println!("before: {sql}"))
    .after(|sql, elapsed| println!("after: {sql} {elapsed:?}"))
    .on_error(|sql, err| eprintln!("error: {sql} {err}"))
    .slow_sql_threshold(std::time::Duration::from_millis(100))
    .slow(|sql, elapsed| eprintln!("slow: {sql} {elapsed:?}"));
```

需要访问参数时使用 `before_with`、`after_with` 或 `on_error_with`。参数脱敏只影响回调视图，不改变真实绑定值。

```rust
db.sql_trace()
    .redact_params(|params| {
        params.iter().map(|_| ormer::Value::Text("***".into())).collect()
    })
    .before_with(|event| {
        println!("sql={} params={:?}", event.sql(), event.params());
    });
```

## HookContext

`HookContext` 提供：

- `operation()`：当前操作类型，取值为 `Insert`、`Update` 或 `Delete`。
- `batch_index()`：批量操作中的记录索引；单条操作为 `None`。
- `in_transaction()`：当前操作是否由 `Transaction` 执行。
