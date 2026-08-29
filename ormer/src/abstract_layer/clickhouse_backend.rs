use crate::migration::{MIGRATION_TABLE_NAME, Migration, MigrationInfo};
use crate::model::DbBackendTypeMapper;
use crate::raw_sql::IntoRawSql;
use serde::Serialize;
use serde::ser::Serializer;

/// ClickHouse SQL type mapping for schema generation and SQL rendering.
pub struct ClickHouseTypeMapper;

/// Minimal ClickHouse HTTP database handle.
///
/// ClickHouse does not expose transactions or row values through the same
/// driver contract as the row-oriented backends. This handle intentionally
/// covers the backend-native DDL and raw SQL operations that are safe to
/// provide without pretending those features exist.
#[derive(Clone)]
pub struct Database {
    client: clickhouse::Client,
}

impl Database {
    /// Connect to a ClickHouse HTTP endpoint.
    ///
    /// The optional `database` query parameter is copied to the client
    /// configuration because the ClickHouse client replaces URL query
    /// parameters with request settings.
    pub(crate) fn connect(connection_string: &str) -> crate::Result<Self> {
        let options = parse_connection_string(connection_string)?;
        let mut client = clickhouse::Client::default().with_url(options.url);
        if let Some(database) = options.database {
            client = client.with_database(database);
        }
        if options.access_token.is_some() && (options.user.is_some() || options.password.is_some())
        {
            return Err(crate::ormer_error!(
                "ClickHouse connection string cannot combine access_token with user or password"
            ));
        }
        if let Some(access_token) = options.access_token {
            client = client.with_access_token(access_token);
        } else {
            if let Some(user) = options.user {
                client = client.with_user(user);
            }
            if let Some(password) = options.password {
                client = client.with_password(password);
            }
        }
        if let Some(compression) = options.compression {
            client = client.with_compression(compression);
        }
        for (name, value) in options.settings {
            client = client.with_setting(name, value);
        }
        Ok(Self { client })
    }

    pub(crate) async fn select_values(
        &self,
        sql: impl IntoRawSql,
        columns: Option<&[&str]>,
    ) -> crate::Result<Vec<Vec<crate::model::Value>>> {
        let rows = self.select_json(sql).await?;
        rows.into_iter()
            .map(|row| clickhouse_row_values(&row, columns))
            .collect()
    }

    /// Execute a raw ClickHouse statement without bound parameters.
    pub(crate) async fn execute_sql(&self, sql: impl IntoRawSql) -> crate::Result<()> {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(crate::abstract_layer::DbType::ClickHouse)?;

        let trace = crate::sql_trace::start_sql_trace(&sql, &params);
        let mut query = self.client.query(&sql);
        for param in &params {
            query = query.bind(clickhouse_bind_value(param));
        }
        match query.execute().await {
            Ok(()) => {
                trace.finish_ok();
                Ok(())
            }
            Err(error) => Err(trace.finish_external_error("clickhouse::Client::query", error)),
        }
    }

    /// Execute a SELECT query and decode its `JSONEachRow` response.
    ///
    /// This backend-native dynamic API stays separate from the unified ORM
    /// executors because ClickHouse row decoding requires a static
    /// `clickhouse::Row` type.
    pub(crate) async fn select_json(
        &self,
        sql: impl IntoRawSql,
    ) -> crate::Result<Vec<serde_json::Value>> {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(crate::abstract_layer::DbType::ClickHouse)?;

        let trace = crate::sql_trace::start_sql_trace(&sql, &params);
        let mut query = self.client.query(&sql);
        for param in &params {
            query = query.bind(clickhouse_bind_value(param));
        }

        let mut cursor = match query.fetch_bytes("JSONEachRow") {
            Ok(cursor) => cursor,
            Err(error) => {
                return Err(trace.finish_external_error("clickhouse::Query::fetch_bytes", error));
            }
        };
        let bytes = match cursor.collect().await {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(trace.finish_external_error("clickhouse::BytesCursor::collect", error));
            }
        };
        let result = match parse_json_each_row(&bytes) {
            Ok(result) => result,
            Err(error) => return Err(trace.finish_error(error)),
        };
        trace.finish_ok();
        Ok(result)
    }

    /// Execute a SELECT query and decode rows using ClickHouse's native
    /// RowBinary decoder.
    pub(crate) async fn select<T>(&self, sql: impl IntoRawSql) -> crate::Result<Vec<T>>
    where
        T: clickhouse::RowOwned + clickhouse::RowRead,
    {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(crate::abstract_layer::DbType::ClickHouse)?;

        let trace = crate::sql_trace::start_sql_trace(&sql, &params);
        let mut query = self.client.query(&sql);
        for param in &params {
            query = query.bind(clickhouse_bind_value(param));
        }
        match query.fetch_all::<T>().await {
            Ok(rows) => {
                trace.finish_ok();
                Ok(rows)
            }
            Err(error) => Err(trace.finish_external_error("clickhouse::Query::fetch_all", error)),
        }
    }

    /// Execute a SELECT query and return a streaming RowBinary cursor.
    pub(crate) fn select_stream<T>(
        &self,
        sql: impl IntoRawSql,
    ) -> crate::Result<clickhouse::query::RowCursor<T>>
    where
        T: clickhouse::Row,
    {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(crate::abstract_layer::DbType::ClickHouse)?;
        let mut query = self.client.query(&sql);
        for param in &params {
            query = query.bind(clickhouse_bind_value(param));
        }
        query
            .fetch::<T>()
            .map_err(|error| crate::OrmerError::from_external("clickhouse::Query::fetch", error))
    }

    /// Execute a SELECT query and return a streaming JSONEachRow cursor.
    pub(crate) fn select_json_stream(
        &self,
        sql: impl IntoRawSql,
    ) -> crate::Result<clickhouse::query::BytesCursor> {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(crate::abstract_layer::DbType::ClickHouse)?;
        let mut query = self.client.query(&sql);
        for param in &params {
            query = query.bind(clickhouse_bind_value(param));
        }
        query.fetch_bytes("JSONEachRow").map_err(|error| {
            crate::OrmerError::from_external("clickhouse::Query::fetch_bytes", error)
        })
    }

    /// Execute a SELECT query and return at most one row.
    pub(crate) async fn select_optional<T>(&self, sql: impl IntoRawSql) -> crate::Result<Option<T>>
    where
        T: clickhouse::RowOwned + clickhouse::RowRead,
    {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(crate::abstract_layer::DbType::ClickHouse)?;
        let mut query = self.client.query(&sql);
        for param in &params {
            query = query.bind(clickhouse_bind_value(param));
        }
        query.fetch_optional::<T>().await.map_err(|error| {
            crate::OrmerError::from_external("clickhouse::Query::fetch_optional", error)
        })
    }

    /// Execute a SELECT query and decode one row using ClickHouse's native
    /// RowBinary decoder.
    pub(crate) async fn select_one<T>(&self, sql: impl IntoRawSql) -> crate::Result<T>
    where
        T: clickhouse::RowOwned + clickhouse::RowRead,
    {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(crate::abstract_layer::DbType::ClickHouse)?;

        let trace = crate::sql_trace::start_sql_trace(&sql, &params);
        let mut query = self.client.query(&sql);
        for param in &params {
            query = query.bind(clickhouse_bind_value(param));
        }
        match query.fetch_one::<T>().await {
            Ok(row) => {
                trace.finish_ok();
                Ok(row)
            }
            Err(error) => Err(trace.finish_external_error("clickhouse::Query::fetch_one", error)),
        }
    }

    /// Insert typed rows using ClickHouse's native RowBinary protocol.
    pub(crate) async fn insert_rows<T, I>(&self, table: &str, rows: I) -> crate::Result<()>
    where
        T: clickhouse::RowOwned + clickhouse::RowWrite,
        I: IntoIterator<Item = T>,
    {
        let table = crate::model::quote_qualified_identifier(
            crate::abstract_layer::DbType::ClickHouse,
            table,
        );
        let mut insert = self
            .client
            .insert_unescaped::<T>(&table)
            .await
            .map_err(|error| {
                crate::OrmerError::from_external("clickhouse::Client::insert", error)
            })?;
        for row in rows {
            insert.write(&row).await.map_err(|error| {
                crate::OrmerError::from_external("clickhouse::Insert::write", error)
            })?;
        }
        insert
            .end()
            .await
            .map_err(|error| crate::OrmerError::from_external("clickhouse::Insert::end", error))
    }

    /// Check whether the ClickHouse endpoint accepts a trivial query.
    pub(crate) async fn is_valid(&self) -> bool {
        self.select_json("SELECT 1").await.is_ok()
    }

    /// Generate and execute a ClickHouse CREATE TABLE statement.
    pub(crate) async fn create_table<T: crate::model::WritableModel>(
        &self,
        engine: &str,
    ) -> crate::Result<()> {
        let sql = crate::generate_clickhouse_create_table_sql::<T>(engine)?;
        self.execute_sql(crate::raw_sql::RawSql::plain(sql)).await
    }

    /// Drop a model table if it exists.
    pub(crate) async fn drop_table<T: crate::model::WritableModel>(&self) -> crate::Result<()> {
        let table = crate::model::quote_qualified_identifier(
            crate::abstract_layer::DbType::ClickHouse,
            T::table_name_for_db(crate::abstract_layer::DbType::ClickHouse),
        );
        self.execute_sql(crate::raw_sql::RawSql::plain(format!(
            "DROP TABLE IF EXISTS {table}"
        )))
        .await
    }

    /// Generate Rust model definitions from ClickHouse system metadata.
    ///
    /// ClickHouse databases are treated as the schema selector. When omitted,
    /// the database configured on this client is used.
    pub(crate) async fn generate_entities(&self, schema: Option<&str>) -> crate::Result<String> {
        let tables = self.db_first_tables(schema).await?;
        Ok(crate::db_first::generate_entities(
            crate::abstract_layer::DbType::ClickHouse,
            &tables,
        ))
    }

    pub(crate) async fn db_first_tables(
        &self,
        schema: Option<&str>,
    ) -> crate::Result<Vec<crate::DbFirstTable>> {
        let database_filter = schema
            .filter(|schema| !schema.trim().is_empty())
            .map(str::to_string);
        let query = if database_filter.is_some() {
            crate::raw_sql::RawSql::new(
                "SELECT database, name \
                 FROM system.tables \
                 WHERE database = {} \
                   AND is_temporary = 0 \
                   AND database != 'system' \
                   AND name != '__ormer_migrations' \
                 ORDER BY name",
            )
            .bind(database_filter.clone().expect("database filter is present"))
        } else {
            crate::raw_sql::RawSql::plain(
                "SELECT database, name \
                 FROM system.tables \
                 WHERE database = currentDatabase() \
                   AND is_temporary = 0 \
                   AND database != 'system' \
                   AND name != '__ormer_migrations' \
                 ORDER BY name",
            )
        };
        let table_rows = self.select_json(query).await?;

        let mut tables = Vec::with_capacity(table_rows.len());
        for table_row in table_rows {
            let database = table_row
                .get("database")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| crate::ormer_error!("Invalid ClickHouse table metadata"))?;
            let table_name = table_row
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| crate::ormer_error!("Invalid ClickHouse table metadata"))?;
            let columns = self
                .clickhouse_db_first_columns(database, table_name)
                .await?;
            tables.push(crate::DbFirstTable {
                schema: Some(database.to_string()),
                name: table_name.to_string(),
                columns,
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
            });
        }
        Ok(tables)
    }

    async fn clickhouse_db_first_columns(
        &self,
        database: &str,
        table: &str,
    ) -> crate::Result<Vec<crate::DbFirstColumn>> {
        let rows = self
            .select_json(
                crate::raw_sql::RawSql::new(
                    "SELECT name, type, default_expression, is_in_primary_key \
                     FROM system.columns \
                     WHERE database = {} AND table = {} \
                     ORDER BY position",
                )
                .bind(database)
                .bind(table),
            )
            .await?;
        rows.into_iter()
            .map(parse_clickhouse_db_first_column)
            .collect()
    }

    /// Read the native ClickHouse migration history table.
    pub(crate) async fn migration_history(&self) -> crate::Result<Vec<MigrationInfo>> {
        self.ensure_migration_table().await?;
        let table = crate::model::quote_identifier(
            crate::abstract_layer::DbType::ClickHouse,
            MIGRATION_TABLE_NAME,
        );
        let rows = self
            .select_json(format!(
                "SELECT version, name, checksum FROM {table} ORDER BY version"
            ))
            .await?;
        rows.into_iter().map(parse_migration_info).collect()
    }

    /// Return migrations that are not present in ClickHouse's history table.
    pub(crate) async fn pending_migrations<M: Migration>(
        &self,
        migrations: &[M],
    ) -> crate::Result<Vec<MigrationInfo>> {
        let applied = self
            .migration_history()
            .await?
            .into_iter()
            .map(|migration| (migration.version, migration.checksum))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut sorted = migrations.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|migration| migration.version());
        let mut seen = std::collections::BTreeSet::new();
        let mut pending = Vec::new();
        for migration in sorted {
            if !seen.insert(migration.version()) {
                return Err(crate::ormer_error!(
                    "Duplicate migration version {}",
                    migration.version()
                ));
            }
            if let Some(checksum) = applied.get(&migration.version()) {
                if *checksum != migration.checksum() {
                    return Err(crate::ormer_error!(
                        "Migration {} checksum changed after it was applied",
                        migration.version()
                    ));
                }
                continue;
            }
            pending.push(MigrationInfo {
                version: migration.version(),
                name: migration.name().to_string(),
                checksum: migration.checksum(),
            });
        }
        Ok(pending)
    }

    /// Apply native ClickHouse migrations one statement at a time.
    ///
    /// ClickHouse DDL is not transactional. If a later step fails, earlier
    /// steps remain applied and the migration is not recorded as complete.
    pub(crate) async fn apply_migrations<M: Migration>(
        &self,
        migrations: &[M],
    ) -> crate::Result<usize> {
        let pending = self.pending_migrations(migrations).await?;
        if pending.is_empty() {
            return Ok(0);
        }

        let mut by_version = migrations
            .iter()
            .map(|migration| (migration.version(), migration))
            .collect::<std::collections::BTreeMap<_, _>>();
        let table = crate::model::quote_identifier(
            crate::abstract_layer::DbType::ClickHouse,
            MIGRATION_TABLE_NAME,
        );
        for migration in &pending {
            let definition = by_version
                .remove(&migration.version)
                .ok_or_else(|| crate::ormer_error!("Migration definition disappeared"))?;
            for step in definition.up() {
                let sql = step.sql(crate::abstract_layer::DbType::ClickHouse)?;
                if sql.contains(';') {
                    return Err(crate::ormer_error!(
                        "ClickHouse migration steps must contain one statement"
                    ));
                }
                self.execute_sql(crate::raw_sql::RawSql::plain(sql)).await?;
            }
            let name = migration.name.replace('\'', "''");
            self.execute_sql(crate::raw_sql::RawSql::plain(format!(
                "INSERT INTO {table} (version, name, checksum) VALUES ({}, '{}', {})",
                migration.version, name, migration.checksum
            )))
            .await?;
        }
        Ok(pending.len())
    }

    pub(crate) async fn ensure_migration_table(&self) -> crate::Result<()> {
        let table = crate::model::quote_identifier(
            crate::abstract_layer::DbType::ClickHouse,
            MIGRATION_TABLE_NAME,
        );
        self.execute_sql(crate::raw_sql::RawSql::plain(format!(
            "CREATE TABLE IF NOT EXISTS {table} \
             (version UInt64, name String, checksum UInt64, \
              applied_at DateTime64(3) DEFAULT now64(3)) \
             ENGINE = MergeTree ORDER BY version"
        )))
        .await
    }
}

fn clickhouse_row_values(
    row: &serde_json::Value,
    columns: Option<&[&str]>,
) -> crate::Result<Vec<crate::model::Value>> {
    let object = row
        .as_object()
        .ok_or_else(|| crate::ormer_error!("ClickHouse row is not a JSON object"))?;
    let Some(columns) = columns else {
        if object.len() == 1 {
            return Ok(vec![clickhouse_json_value(
                object.values().next().expect("length checked"),
            )?]);
        }
        return Err(crate::ormer_error!(
            "ClickHouse raw SQL requires a single-column result or a ViewModel/Model target"
        ));
    };

    columns
        .iter()
        .map(|column| {
            object
                .get(*column)
                .map(clickhouse_json_value)
                .transpose()?
                .ok_or_else(|| crate::ormer_error!("Missing ClickHouse column: {column}"))
        })
        .collect()
}

fn clickhouse_json_value(value: &serde_json::Value) -> crate::Result<crate::model::Value> {
    use crate::model::Value;
    use serde_json::Value as Json;

    match value {
        Json::Null => Ok(Value::Null),
        Json::Bool(value) => Ok(Value::Boolean(*value)),
        Json::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(Value::Integer(value))
            } else if let Some(value) = value.as_u64() {
                Ok(Value::BigInt(value as i128))
            } else if let Some(value) = value.as_f64() {
                Ok(Value::Real(value))
            } else {
                Ok(Value::Decimal(value.to_string()))
            }
        }
        Json::String(value) => Ok(Value::Text(value.clone())),
        Json::Array(values) => {
            let values = values
                .iter()
                .map(clickhouse_json_value)
                .collect::<crate::Result<Vec<_>>>()?;
            let contains_null = values.iter().any(|value| matches!(value, Value::Null));
            if values
                .iter()
                .all(|value| matches!(value, Value::Integer(_) | Value::BigInt(_) | Value::Null))
            {
                let integers = values
                    .iter()
                    .map(|value| match value {
                        Value::Integer(value) => Ok(Some(i64::from(*value))),
                        Value::BigInt(value) => i64::try_from(*value).map(Some).map_err(|_| {
                            crate::ormer_error!(
                                "ClickHouse integer array value is out of i64 range"
                            )
                        }),
                        Value::Null => Ok(None),
                        _ => unreachable!("integer array checked"),
                    })
                    .collect::<crate::Result<Vec<Option<i64>>>>()?;
                if !contains_null
                    && integers
                        .iter()
                        .all(|value| i32::try_from(value.expect("non-null checked")).is_ok())
                {
                    return Ok(Value::IntegerArray(
                        integers
                            .into_iter()
                            .map(|value| value.expect("non-null checked") as i32)
                            .collect(),
                    ));
                }
                return Ok(Value::NullableBigIntArray(integers));
            }
            if values.iter().all(|value| matches!(value, Value::Text(_))) {
                return Ok(Value::TextArray(
                    values
                        .into_iter()
                        .map(|value| match value {
                            Value::Text(value) => value,
                            _ => unreachable!(" text array checked"),
                        })
                        .collect(),
                ));
            }
            Ok(Value::Json(serde_json::Value::Array(
                values.into_iter().map(model_value_to_json).collect(),
            )))
        }
        Json::Object(value) => Ok(Value::Json(serde_json::Value::Object(value.clone()))),
    }
}

fn model_value_to_json(value: crate::model::Value) -> serde_json::Value {
    use crate::model::Value;

    match value {
        Value::Null => serde_json::Value::Null,
        Value::Integer(value) => serde_json::Value::from(value),
        Value::BigInt(value) => serde_json::Value::String(value.to_string()),
        Value::Duration(value) => serde_json::Value::from(value.as_micros() as u64),
        Value::Text(value) => serde_json::Value::from(value),
        Value::TextArray(value) => {
            serde_json::Value::Array(value.into_iter().map(serde_json::Value::from).collect())
        }
        Value::Real(value) => serde_json::Value::from(value),
        Value::Decimal(value) | Value::BigDecimal(value) => serde_json::Value::from(value),
        Value::Boolean(value) => serde_json::Value::from(value),
        Value::Bytes(value) => {
            serde_json::Value::Array(value.into_iter().map(serde_json::Value::from).collect())
        }
        Value::IntegerArray(value) => {
            serde_json::Value::Array(value.into_iter().map(serde_json::Value::from).collect())
        }
        Value::BigIntArray(value) => {
            serde_json::Value::Array(value.into_iter().map(serde_json::Value::from).collect())
        }
        Value::NullableBigIntArray(value) => serde_json::Value::Array(
            value
                .into_iter()
                .map(|value| value.map_or(serde_json::Value::Null, serde_json::Value::from))
                .collect(),
        ),
        Value::DateTime(value) => serde_json::Value::from(value.to_rfc3339()),
        Value::Date(value) => serde_json::Value::from(value.to_string()),
        Value::Time(value) => serde_json::Value::from(value.to_string()),
        Value::Json(value) => value,
        Value::Uuid(value) => serde_json::Value::from(value.to_string()),
    }
}

fn parse_clickhouse_db_first_column(row: serde_json::Value) -> crate::Result<crate::DbFirstColumn> {
    let name = row
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| crate::ormer_error!("Invalid ClickHouse column name"))?
        .to_string();
    let type_name = row
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| crate::ormer_error!("Invalid ClickHouse column type"))?
        .to_string();
    let default_expression = row
        .get("default_expression")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let primary_key = row
        .get("is_in_primary_key")
        .and_then(json_bool)
        .unwrap_or(false);
    Ok(crate::DbFirstColumn {
        name,
        type_name: type_name.clone(),
        nullable: clickhouse_type_is_nullable(&type_name),
        primary_key,
        auto_increment: false,
        enum_variants: clickhouse_enum_variants(&type_name),
        default: default_expression,
    })
}

fn json_bool(value: &serde_json::Value) -> Option<bool> {
    value.as_bool().or_else(|| {
        value.as_u64().map(|value| value != 0).or_else(|| {
            value
                .as_str()
                .and_then(|value| value.parse::<u64>().ok())
                .map(|value| value != 0)
        })
    })
}

fn clickhouse_type_is_nullable(type_name: &str) -> bool {
    type_name
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("nullable(")
}

fn clickhouse_enum_variants(type_name: &str) -> Vec<String> {
    let type_name = type_name.trim();
    let type_name = type_name
        .strip_prefix("Nullable(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(type_name);
    let Some(open) = type_name.find('(') else {
        return Vec::new();
    };
    if !type_name[..open].trim().eq_ignore_ascii_case("enum8")
        && !type_name[..open].trim().eq_ignore_ascii_case("enum16")
    {
        return Vec::new();
    }
    let Some(close) = type_name.rfind(')') else {
        return Vec::new();
    };
    type_name[open + 1..close]
        .split(',')
        .filter_map(|entry| entry.split_once('='))
        .map(|(name, _)| name.trim().trim_matches('\'').trim_matches('"'))
        .filter(|name| !name.is_empty() && is_plain_rust_ident(name))
        .map(str::to_string)
        .collect()
}

fn is_plain_rust_ident(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn parse_migration_info(row: serde_json::Value) -> crate::Result<MigrationInfo> {
    let version = parse_json_u64(row.get("version"), "version")?;
    let name = row
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| crate::ormer_error!("Invalid ClickHouse migration name"))?
        .to_string();
    let checksum = parse_json_u64(row.get("checksum"), "checksum")?;
    Ok(MigrationInfo {
        version,
        name,
        checksum,
    })
}

fn parse_json_u64(value: Option<&serde_json::Value>, field: &str) -> crate::Result<u64> {
    match value {
        Some(serde_json::Value::Number(value)) => value
            .as_u64()
            .ok_or_else(|| crate::ormer_error!("Invalid ClickHouse migration {field}")),
        Some(serde_json::Value::String(value)) => value
            .parse::<u64>()
            .map_err(|_| crate::ormer_error!("Invalid ClickHouse migration {field}")),
        _ => Err(crate::ormer_error!("Invalid ClickHouse migration {field}")),
    }
}

enum ClickHouseBindValue {
    Integer(i64),
    BigInt(i128),
    Duration(u64),
    Text(String),
    TextArray(Vec<String>),
    Real(f64),
    Decimal(String),
    Boolean(bool),
    Bytes(Vec<u8>),
    IntegerArray(Vec<i32>),
    BigIntArray(Vec<i64>),
    NullableBigIntArray(Vec<Option<i64>>),
    DateTime(String),
    Date(String),
    Time(String),
    Json(String),
    Uuid(String),
    Null,
}

impl Serialize for ClickHouseBindValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Integer(value) => serializer.serialize_i64(*value),
            Self::BigInt(value) => serializer.serialize_i128(*value),
            Self::Duration(value) => serializer.serialize_u64(*value),
            Self::Text(value) => serializer.serialize_str(value),
            Self::TextArray(value) => value.serialize(serializer),
            Self::Real(value) => serializer.serialize_f64(*value),
            Self::Decimal(value) => serializer.serialize_str(value),
            Self::Boolean(value) => serializer.serialize_bool(*value),
            Self::Bytes(value) => serializer.serialize_bytes(value),
            Self::IntegerArray(value) => value.serialize(serializer),
            Self::BigIntArray(value) => value.serialize(serializer),
            Self::NullableBigIntArray(value) => value.serialize(serializer),
            Self::DateTime(value) | Self::Date(value) | Self::Time(value) => {
                serializer.serialize_str(value)
            }
            Self::Json(value) => serializer.serialize_str(value),
            Self::Uuid(value) => serializer.serialize_str(value),
            Self::Null => serializer.serialize_none(),
        }
    }
}

fn clickhouse_bind_value(value: &crate::model::Value) -> ClickHouseBindValue {
    use crate::model::Value;

    match value {
        Value::Integer(value) => ClickHouseBindValue::Integer(*value),
        Value::BigInt(value) => ClickHouseBindValue::BigInt(*value),
        Value::Duration(value) => ClickHouseBindValue::Duration(value.as_micros() as u64),
        Value::Text(value) => ClickHouseBindValue::Text(value.clone()),
        Value::TextArray(value) => ClickHouseBindValue::TextArray(value.clone()),
        Value::Real(value) => ClickHouseBindValue::Real(*value),
        Value::Decimal(value) | Value::BigDecimal(value) => {
            ClickHouseBindValue::Decimal(value.clone())
        }
        Value::Boolean(value) => ClickHouseBindValue::Boolean(*value),
        Value::Bytes(value) => ClickHouseBindValue::Bytes(value.clone()),
        Value::IntegerArray(value) => ClickHouseBindValue::IntegerArray(value.clone()),
        Value::BigIntArray(value) => ClickHouseBindValue::BigIntArray(value.clone()),
        Value::NullableBigIntArray(value) => {
            ClickHouseBindValue::NullableBigIntArray(value.clone())
        }
        Value::DateTime(value) => ClickHouseBindValue::DateTime(value.to_rfc3339()),
        Value::Date(value) => ClickHouseBindValue::Date(value.to_string()),
        Value::Time(value) => ClickHouseBindValue::Time(value.to_string()),
        Value::Json(value) => ClickHouseBindValue::Json(value.to_string()),
        Value::Uuid(value) => ClickHouseBindValue::Uuid(value.to_string()),
        Value::Null => ClickHouseBindValue::Null,
    }
}

struct ClickHouseConnectionOptions {
    url: String,
    database: Option<String>,
    user: Option<String>,
    password: Option<String>,
    access_token: Option<String>,
    compression: Option<clickhouse::Compression>,
    settings: Vec<(String, String)>,
}

fn parse_connection_string(connection_string: &str) -> crate::Result<ClickHouseConnectionOptions> {
    let (url, query) = connection_string
        .split_once('?')
        .map_or((connection_string, ""), |(url, query)| (url, query));
    let mut options = ClickHouseConnectionOptions {
        url: url.to_string(),
        database: None,
        user: None,
        password: None,
        access_token: None,
        compression: None,
        settings: Vec::new(),
    };

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(raw_key)?;
        let value = percent_decode(raw_value)?;
        match key.as_str() {
            "database" => options.database = (!value.is_empty()).then_some(value),
            "user" => options.user = Some(value),
            "password" => options.password = Some(value),
            "access_token" => options.access_token = Some(value),
            "compression" | "compress" => {
                options.compression = Some(parse_compression(&value)?);
            }
            _ => options.settings.push((key, value)),
        }
    }

    Ok(options)
}

fn parse_compression(value: &str) -> crate::Result<clickhouse::Compression> {
    match value.to_ascii_lowercase().as_str() {
        "0" | "none" | "false" | "off" => Ok(clickhouse::Compression::None),
        "1" | "lz4" | "true" | "on" => Ok(clickhouse::Compression::Lz4),
        _ => Err(crate::ormer_error!(
            "Invalid ClickHouse compression setting: {value}"
        )),
    }
}

fn percent_decode(value: &str) -> crate::Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' => {
                let high = bytes
                    .get(index + 1)
                    .and_then(|byte| hex_digit(*byte))
                    .ok_or_else(|| crate::ormer_error!("Invalid ClickHouse URL encoding"))?;
                let low = bytes
                    .get(index + 2)
                    .and_then(|byte| hex_digit(*byte))
                    .ok_or_else(|| crate::ormer_error!("Invalid ClickHouse URL encoding"))?;
                decoded.push((high << 4) | low);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| crate::ormer_error!("Invalid UTF-8 in ClickHouse URL query"))
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_json_each_row(bytes: &[u8]) -> crate::Result<Vec<serde_json::Value>> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| crate::ormer_error!("Invalid ClickHouse JSONEachRow response: {error}"))?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|error| crate::ormer_error!("Invalid ClickHouse JSONEachRow row: {error}"))
        })
        .collect()
}

impl DbBackendTypeMapper for ClickHouseTypeMapper {
    fn sql_type(
        rust_type: &str,
        _is_primary: bool,
        _is_auto_increment: bool,
        is_nullable: bool,
        enum_variants: Option<&[&str]>,
    ) -> String {
        if enum_variants.is_some() {
            return nullable_type("String", is_nullable);
        }
        let base = match rust_type {
            "i8" => "Int8",
            "i16" => "Int16",
            "i32" => "Int32",
            "i64" => "Int64",
            "i128" => "Int128",
            "u8" => "UInt8",
            "u16" => "UInt16",
            "u32" => "UInt32",
            "u64" => "UInt64",
            "u128" => "UInt128",
            "f32" => "Float32",
            "f64" => "Float64",
            "bool" => "UInt8",
            "Vec<u8>" | "&[u8]" => "String",
            "Vec<i32>" | "std::vec::Vec<i32>" | "alloc::vec::Vec<i32>" => "Array(Int32)",
            "Vec<i64>" | "std::vec::Vec<i64>" | "alloc::vec::Vec<i64>" => "Array(Int64)",
            "Vec<Option<i64>>" | "std::vec::Vec<Option<i64>>" | "alloc::vec::Vec<Option<i64>>" => {
                "Array(Nullable(Int64))"
            }
            "Vec<String>" | "std::vec::Vec<String>" | "alloc::vec::Vec<String>" => "Array(String)",
            "DateTime" | "chrono::DateTime" | "chrono::DateTime<chrono::Utc>" => "DateTime64(3)",
            "NaiveDateTime" | "chrono::NaiveDateTime" => "DateTime64(3)",
            "NaiveDate" | "chrono::NaiveDate" => "Date32",
            "NaiveTime" | "chrono::NaiveTime" => "String",
            "Uuid" | "uuid::Uuid" => "UUID",
            "JsonValue" | "serde_json::Value" => "String",
            "Decimal" | "rust_decimal::Decimal" => "Decimal128(38)",
            "BigDecimal" | "bigdecimal::BigDecimal" => "String",
            "Duration" | "std::time::Duration" => "Int64",
            _ => "String",
        };
        nullable_type(base, is_nullable)
    }
}

fn nullable_type(base: &str, nullable: bool) -> String {
    if nullable {
        format!("Nullable({base})")
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_clickhouse_db_first_column, parse_compression, parse_connection_string,
        parse_json_each_row, parse_migration_info,
    };

    #[test]
    fn parses_json_each_row() {
        let rows = parse_json_each_row(
            br#"{"id":1}
{"id":2}
"#,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["id"], 2);
    }

    #[test]
    fn parses_migration_history_rows() {
        let row = serde_json::json!({
            "version": "7",
            "name": "add_email",
            "checksum": 42
        });
        let migration = parse_migration_info(row).unwrap();
        assert_eq!(migration.version, 7);
        assert_eq!(migration.name, "add_email");
        assert_eq!(migration.checksum, 42);
    }

    #[test]
    fn parses_connection_options() {
        let options = parse_connection_string(
            "http://localhost:8123?database=analytics%20db&user=reporting&password=p%40ss\
             &compression=none&max_execution_time=3",
        )
        .unwrap();
        assert_eq!(options.url, "http://localhost:8123");
        assert_eq!(options.database.as_deref(), Some("analytics db"));
        assert_eq!(options.user.as_deref(), Some("reporting"));
        assert_eq!(options.password.as_deref(), Some("p@ss"));
        assert_eq!(options.compression, Some(clickhouse::Compression::None));
        assert_eq!(
            options.settings,
            vec![("max_execution_time".to_string(), "3".to_string())]
        );
    }

    #[test]
    fn parses_access_token_and_compression_alias() {
        let options =
            parse_connection_string("http://localhost:8123?access_token=jwt&compress=1").unwrap();
        assert_eq!(options.access_token.as_deref(), Some("jwt"));
        assert_eq!(options.compression, Some(clickhouse::Compression::Lz4));
    }

    #[test]
    fn rejects_invalid_compression() {
        assert!(parse_compression("gzip").is_err());
    }

    #[test]
    fn parses_clickhouse_db_first_column_metadata() {
        let column = parse_clickhouse_db_first_column(serde_json::json!({
            "name": "tags",
            "type": "Nullable(Array(Int64))",
            "default_expression": "",
            "is_in_primary_key": "0",
        }))
        .unwrap();
        assert_eq!(column.name, "tags");
        assert_eq!(column.type_name, "Nullable(Array(Int64))");
        assert!(column.nullable);
        assert!(!column.primary_key);
        assert_eq!(column.default, None);
    }

    #[test]
    fn parses_clickhouse_enum_column_metadata() {
        let column = parse_clickhouse_db_first_column(serde_json::json!({
            "name": "state",
            "type": "Nullable(Enum8('Draft' = 1, 'Published' = 2))",
            "default_expression": "'Draft'",
            "is_in_primary_key": 0,
        }))
        .unwrap();
        assert!(column.nullable);
        assert_eq!(column.enum_variants, vec!["Draft", "Published"]);
    }

    #[test]
    fn clickhouse_db_first_generates_native_types() {
        let code = crate::db_first::generate_entities(
            crate::abstract_layer::DbType::ClickHouse,
            &[crate::DbFirstTable {
                schema: Some("analytics".to_string()),
                name: "events".to_string(),
                columns: vec![
                    crate::DbFirstColumn {
                        name: "id".to_string(),
                        type_name: "UInt64".to_string(),
                        nullable: false,
                        primary_key: true,
                        auto_increment: false,
                        enum_variants: Vec::new(),
                        default: None,
                    },
                    crate::DbFirstColumn {
                        name: "tags".to_string(),
                        type_name: "Array(Int32)".to_string(),
                        nullable: false,
                        primary_key: false,
                        auto_increment: false,
                        enum_variants: Vec::new(),
                        default: None,
                    },
                    crate::DbFirstColumn {
                        name: "score".to_string(),
                        type_name: "Nullable(Float64)".to_string(),
                        nullable: true,
                        primary_key: false,
                        auto_increment: false,
                        enum_variants: Vec::new(),
                        default: None,
                    },
                ],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
            }],
        );
        assert!(code.contains("pub id: u64"), "{code}");
        assert!(code.contains("pub tags: Vec<i32>"), "{code}");
        assert!(code.contains("pub score: Option<f64>"), "{code}");
    }
}
