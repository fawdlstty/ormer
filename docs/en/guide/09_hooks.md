# Hooks System

Hooks provide fallible lifecycle callbacks for write operations. Every callback receives a `HookContext` and returns `ormer::Result<()>`; validation should return an error instead of panicking.

| Hook Trait | Timing | Signature |
| --- | --- | --- |
| `BeforeInsert` | Before insert SQL | `async fn before_insert(&mut self, ctx: &mut HookContext<'_>) -> Result<()>` |
| `AfterInsert` | After successful insert SQL | `async fn after_insert(&self, ctx: &mut HookContext<'_>) -> Result<()>` |
| `BeforeUpdate` | Before update SQL | `async fn before_update(&mut self, ctx: &mut HookContext<'_>) -> Result<()>` |
| `AfterUpdate` | After an update affects at least one row | `async fn after_update(&self, ctx: &mut HookContext<'_>) -> Result<()>` |
| `BeforeDelete` | Before delete SQL | `async fn before_delete(&self, ctx: &mut HookContext<'_>) -> Result<()>` |
| `AfterDelete` | After a delete affects at least one row | `async fn after_delete(&self, ctx: &mut HookContext<'_>) -> Result<()>` |

## Inserts

Insert hooks use mutable model inputs so `BeforeInsert` can normalize or fill fields. For models implementing `BeforeInsert` and `AfterInsert`, `insert`, `insert_or_update`, `insert_or_ignore`, and their transaction executors run hooks in `BeforeInsert -> SQL -> AfterInsert` order.

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

Pass a mutable collection for a batch insert. Hooks run once per record and `ctx.batch_index()` identifies that record.

```rust
let mut users = vec![user_a, user_b];
db.insert(&mut users).execute().await?;
```

An error from `BeforeInsert` prevents SQL execution. An error from `AfterInsert` is returned after SQL has run; for transaction operations, the caller should call `rollback()` when that is the required business outcome.

## Updates and Deletes

An update or delete without a model cannot identify a hook subject. Use `execute_with_hooks` to supply that model. Use `execute_models_with_hooks` for batches; each context carries its batch index.

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

For an insert inside a transaction, `ctx.in_transaction()` is `true`. Hook failures are returned as `Result` and do not commit the transaction; the caller chooses `commit()` or `rollback()`.

## HookContext

`HookContext` exposes:

- `operation()` for `Insert`, `Update`, or `Delete`.
- `batch_index()` for the index in a batch, or `None` for a single record.
- `in_transaction()` to indicate execution through `Transaction`.
