use crate::abstract_layer::common::common_helpers::{
    resolve_influx_time_key, time_delete_bounds, BlockRange,
};
use crate::abstract_layer::common::BlockDeleteResult;
use crate::migration::{MIGRATION_TABLE_NAME, Migration, MigrationInfo};
use crate::model::{Model, Value};
use crate::raw_sql::IntoRawSql;

/// InfluxDB 服务端版本与对应的认证信息。
#[derive(Clone, Debug)]
pub(crate) enum InfluxMode {
    /// 2.x：bucket + token（走 /api/v2/write 与 /api/v2/delete，查询走 v1 兼容 /query）
    V2 {
        org: String,
        bucket: String,
        token: String,
    },
    /// 1.x：database + 用户名密码
    V1 {
        database: String,
        user: String,
        password: String,
    },
}

/// InfluxDB HTTP 后端句柄。
#[derive(Clone, Debug)]
pub struct Database {
    http: reqwest::Client,
    url: String,
    mode: InfluxMode,
}

#[allow(dead_code)]
impl Database {
    /// 连接串：`http://host:8086?org=..&bucket=..&token=..`（2.x）
    /// 或 `http://host:8086?database=..&user=..&password=..`（1.x）。
    pub(crate) fn connect(connection_string: &str) -> crate::Result<Self> {
        let url = reqwest::Url::parse(connection_string)
            .map_err(|error| crate::OrmerError::from_external("reqwest::Url::parse", error))?;
        let mut org = None;
        let mut bucket = None;
        let mut token = None;
        let mut database = None;
        let mut user = None;
        let mut password = None;
        for (name, value) in url.query_pairs() {
            match name.as_ref() {
                "org" => org = Some(value.into_owned()),
                "bucket" | "db" => bucket = Some(value.into_owned()),
                "token" => token = Some(value.into_owned()),
                "database" => database = Some(value.into_owned()),
                "user" | "username" => user = Some(value.into_owned()),
                "password" => password = Some(value.into_owned()),
                _ => {}
            }
        }
        let non_empty = |value: Option<String>| value.filter(|value| !value.is_empty());
        let mode = if token.is_some() || bucket.is_some() || org.is_some() {
            let org = non_empty(org)
                .ok_or_else(|| crate::ormer_error!("InfluxDB connection string requires org"))?;
            let bucket = non_empty(bucket).ok_or_else(|| {
                crate::ormer_error!("InfluxDB connection string requires bucket")
            })?;
            let token = non_empty(token).ok_or_else(|| {
                crate::ormer_error!("InfluxDB connection string requires token")
            })?;
            InfluxMode::V2 {
                org,
                bucket,
                token,
            }
        } else {
            let database = non_empty(database).ok_or_else(|| {
                crate::ormer_error!(
                    "InfluxDB connection string requires org/bucket/token (2.x) or database (1.x)"
                )
            })?;
            InfluxMode::V1 {
                database,
                user: user.unwrap_or_default(),
                password: password.unwrap_or_default(),
            }
        };
        let mut base = url.clone();
        base.set_query(None);
        base.set_fragment(None);
        Ok(Self {
            http: reqwest::Client::new(),
            url: base.to_string(),
            mode,
        })
    }

    fn database_name(&self) -> &str {
        match &self.mode {
            InfluxMode::V2 { bucket, .. } => bucket,
            InfluxMode::V1 { database, .. } => database,
        }
    }

    fn base_url(&self) -> String {
        self.url.trim_end_matches('/').to_string()
    }

    fn health_path(&self) -> String {
        format!("{}/health", self.base_url())
    }

    /// v1 兼容查询端点：1.x 原生，2.x 通过 bucket 映射提供 InfluxQL。
    fn query_path(&self) -> String {
        format!("{}/query", self.base_url())
    }

    fn write_path(&self) -> String {
        match &self.mode {
            InfluxMode::V2 { .. } => format!("{}/api/v2/write", self.base_url()),
            InfluxMode::V1 { .. } => format!("{}/write", self.base_url()),
        }
    }

    fn delete_path(&self) -> Option<String> {
        match &self.mode {
            InfluxMode::V2 { .. } => Some(format!("{}/api/v2/delete", self.base_url())),
            InfluxMode::V1 { .. } => None,
        }
    }

    fn auth_header(&self) -> Option<String> {
        match &self.mode {
            InfluxMode::V2 { token, .. } => Some(format!("Token {token}")),
            InfluxMode::V1 { .. } => None,
        }
    }

    fn write_query_params(&self) -> Vec<(&'static str, String)> {
        let mut params: Vec<(&'static str, String)> = Vec::new();
        match &self.mode {
            InfluxMode::V2 { org, bucket, .. } => {
                params.push(("org", org.clone()));
                params.push(("bucket", bucket.clone()));
            }
            InfluxMode::V1 {
                database,
                user,
                password,
            } => {
                params.push(("db", database.clone()));
                if !user.is_empty() {
                    params.push(("u", user.clone()));
                }
                if !password.is_empty() {
                    params.push(("p", password.clone()));
                }
            }
        }
        params.push(("precision", "ns".to_string()));
        params
    }

    fn query_request_params(&self, q: &str) -> Vec<(&'static str, String)> {
        let mut params: Vec<(&'static str, String)> =
            vec![("db", self.database_name().to_string()), ("q", q.to_string()), ("epoch", "ns".to_string())];
        if let InfluxMode::V2 { org, .. } = &self.mode {
            params.push(("org", org.clone()));
        }
        if let InfluxMode::V1 {
            user, password, ..
        } = &self.mode
        {
            if !user.is_empty() {
                params.push(("u", user.clone()));
            }
            if !password.is_empty() {
                params.push(("p", password.clone()));
            }
        }
        params
    }

    pub(crate) async fn is_valid(&self) -> bool {
        let mut request = self.http.get(self.health_path());
        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }
        request
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    /// 执行一条 InfluxQL/SQL 查询，返回所有 series（列名与行值）。
    pub(crate) async fn query_influxql(
        &self,
        q: &str,
    ) -> crate::Result<Vec<InfluxSeries>> {
        let mut request = self.http.get(self.query_path());
        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }
        for (name, value) in self.query_request_params(q) {
            request = request.query(&[(name, value)]);
        }
        let response = request.send().await.map_err(|error| {
            crate::OrmerError::from_external("reqwest::Client::send (InfluxDB query)", error)
        })?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(crate::ormer_error!(
                "InfluxDB query failed: {status}: {body}"
            ));
        }
        parse_query_response(&body)
    }

    /// 执行一条语句（DDL/DML），返回受影响行数（InfluxDB 恒为 0）。
    pub(crate) async fn execute_influxql(&self, q: &str) -> crate::Result<u64> {
        let mut request = self.http.post(self.query_path());
        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }
        let mut params = self.query_request_params(q);
        params.retain(|(name, _)| *name != "epoch");
        for (name, value) in params {
            request = request.query(&[(name, value)]);
        }
        let response = request.send().await.map_err(|error| {
            crate::OrmerError::from_external("reqwest::Client::send (InfluxDB statement)", error)
        })?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(crate::ormer_error!(
                "InfluxDB statement failed: {status}: {body}"
            ));
        }
        // 查询结果中可能内嵌 error 字段
        for error in parse_query_errors(&body) {
            return Err(error);
        }
        Ok(0)
    }

    /// Line Protocol 批量写入。
    pub(crate) async fn write_lines(&self, lines: &str) -> crate::Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        let mut request = self
            .http
            .post(self.write_path())
            .header("Content-Type", "text/plain; charset=utf-8");
        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }
        for (name, value) in self.write_query_params() {
            request = request.query(&[(name, value)]);
        }
        let response = request.body(lines.to_string()).send().await.map_err(|error| {
            crate::OrmerError::from_external("reqwest::Client::send (InfluxDB write)", error)
        })?;
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status();
        let message = response
            .text()
            .await
            .unwrap_or_else(|_| "InfluxDB write request failed".to_string());
        Err(crate::ormer_error!(
            "InfluxDB write failed: {status}: {message}"
        ))
    }

    /// 把渲染后的 SQL（`?` 占位符 + 参数）内联成 InfluxQL 文本。
    pub(crate) fn inline_sql(sql: &str, params: &[Value]) -> crate::Result<String> {
        let mut result = String::with_capacity(sql.len() + params.len() * 8);
        let mut param_index = 0;
        let mut chars = sql.chars().peekable();
        let mut in_string = false;
        while let Some(character) = chars.next() {
            match character {
                '\'' => {
                    in_string = !in_string;
                    result.push('\'');
                }
                '?' if !in_string => {
                    let value = params.get(param_index).ok_or_else(|| {
                        crate::ormer_error!("InfluxDB query is missing parameter {param_index}")
                    })?;
                    param_index += 1;
                    result.push_str(&value_to_influxql_literal(value)?);
                }
                _ => result.push(character),
            }
        }
        if param_index < params.len() {
            return Err(crate::ormer_error!(
                "InfluxDB query has {} unused parameters",
                params.len() - param_index
            ));
        }
        Ok(result)
    }

    /// 执行 SELECT 并按请求列返回模型值。
    pub(crate) async fn select_values(
        &self,
        sql: impl IntoRawSql,
        columns: Option<&[&str]>,
    ) -> crate::Result<Vec<Vec<Value>>> {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(crate::abstract_layer::DbType::InfluxDB)?;
        let q = Self::inline_sql(&sql, &params)?;
        let series = self.query_influxql(&q).await?;
        Ok(flatten_series_values(&series, columns)?)
    }

    /// 执行原生语句/查询，返回行值（单列规则与 ClickHouse 一致）。
    pub(crate) async fn raw_select_values(
        &self,
        sql: impl IntoRawSql,
        columns: Option<&[&str]>,
    ) -> crate::Result<Vec<Vec<Value>>> {
        self.select_values(sql, columns).await
    }

    pub(crate) async fn execute_sql(&self, sql: impl IntoRawSql) -> crate::Result<u64> {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(crate::abstract_layer::DbType::InfluxDB)?;
        let q = Self::inline_sql(&sql, &params)?;
        self.execute_influxql(&q).await
    }

    /// 把模型渲染为 Line Protocol 并批量写入。
    /// 同测量 + 同标签 + 同时间戳的重复写入由 InfluxDB 自然覆盖。
    pub(crate) async fn insert_models<T: Model>(&self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }
        validate_influx_model::<T>(crate::abstract_layer::DbType::InfluxDB)?;
        let lines = render_line_protocol(models)?;
        self.write_lines(&lines).await
    }

    /// 建表：无建表 DDL；声明 `#[influxdb(retention = ...)]` 时创建保留策略。
    pub(crate) async fn create_table<T: Model>(&self) -> crate::Result<()> {
        let Some(retention) = T::TABLE_OPTIONS.and_then(|options| options.influxdb_retention)
        else {
            return Ok(());
        };
        let database = quote_influx_identifier(self.database_name());
        let policy = retention_policy_name(T::TABLE_NAME);
        let statement = format!(
            "CREATE RETENTION POLICY {policy} ON {database} \
             DURATION {} REPLICATION 1 DEFAULT",
            format_influx_duration(retention)?
        );
        self.execute_influxql(&statement).await.map(|_| ())
    }

    /// 删除 measurement。
    pub(crate) async fn drop_table<T: Model>(&self) -> crate::Result<()> {
        let measurement = T::table_name_for_db(crate::abstract_layer::DbType::InfluxDB);
        let statement = format!(
            "DROP MEASUREMENT {}",
            quote_influx_identifier(measurement)
        );
        self.execute_influxql(&statement).await.map(|_| ())
    }

    /// 读取 `__ormer_migrations` measurement 中的迁移历史。
    pub(crate) async fn migration_history(&self) -> crate::Result<Vec<MigrationInfo>> {
        let measurement = quote_influx_identifier(MIGRATION_TABLE_NAME);
        let series = self
            .query_influxql(&format!(
                "SELECT version, checksum FROM {measurement} ORDER BY time"
            ))
            .await?;
        let mut migrations = Vec::new();
        for entry in &series {
            let name = entry
                .tags
                .get("name")
                .cloned()
                .unwrap_or_default();
            for row in &entry.rows {
                let version = parse_json_u64(row.get("version"), "version")?;
                let checksum = parse_json_u64(row.get("checksum"), "checksum")?;
                migrations.push(MigrationInfo {
                    version,
                    name: name.clone(),
                    checksum,
                });
            }
        }
        migrations.sort_by_key(|migration| migration.version);
        Ok(migrations)
    }

    /// 迁移逐条执行、失败不回滚；历史记录写入 `__ormer_migrations` measurement。
    pub(crate) async fn apply_migrations<M: Migration>(
        &self,
        migrations: &[M],
    ) -> crate::Result<usize> {
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
        if pending.is_empty() {
            return Ok(0);
        }

        let mut by_version = migrations
            .iter()
            .map(|migration| (migration.version(), migration))
            .collect::<std::collections::BTreeMap<_, _>>();
        for record in &pending {
            let definition = by_version
                .remove(&record.version)
                .ok_or_else(|| crate::ormer_error!("Migration definition disappeared"))?;
            for step in definition.up() {
                let sql = step.sql(crate::abstract_layer::DbType::InfluxDB)?;
                self.execute_influxql(&sql).await?;
            }
            let now = chrono::Utc::now();
            let timestamp = now.timestamp_nanos_opt().unwrap_or_default();
            let line = format!(
                "{},name={} version={}i,checksum={}i {}",
                MIGRATION_TABLE_NAME,
                escape_tag_value(&record.name),
                record.version,
                record.checksum,
                timestamp
            );
            self.write_lines(&line).await?;
        }
        Ok(pending.len())
    }

    pub(crate) fn delete_blocks<T: Model>(&self) -> BlockDeleteExecutor<'_, T> {
        BlockDeleteExecutor::new(self)
    }

    async fn delete_range<T: Model>(
        &self,
        range: BlockRange,
        now: chrono::DateTime<chrono::Utc>,
    ) -> crate::Result<BlockDeleteResult> {
        let Some((start, stop)) = time_delete_bounds(range, now)? else {
            return Ok(BlockDeleteResult::default());
        };
        let Some(delete_path) = self.delete_path() else {
            // 1.x 只能走 InfluxQL DELETE
            let measurement = T::table_name_for_db(crate::abstract_layer::DbType::InfluxDB);
            let predicate = format!("_measurement = '{}'", measurement.replace('\'', "\\'"));
            let start = start
                .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH)
                .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
            let stop = stop.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
            self.execute_influxql(&format!(
                "DELETE FROM {} WHERE time >= '{}' AND time <= '{}'",
                quote_influx_identifier(measurement),
                start,
                stop
            ))
            .await?;
            let _ = predicate;
            return Ok(BlockDeleteResult::default());
        };
        let measurement = T::table_name_for_db(crate::abstract_layer::DbType::InfluxDB);
        let predicate = format!("_measurement = \"{}\"", measurement.replace('"', "\\\""));
        let start = start
            .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH)
            .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        let stop = stop.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        let InfluxMode::V2 { org, bucket, token } = &self.mode else {
            unreachable!("delete path only exists for 2.x");
        };
        let response = self
            .http
            .post(delete_path)
            .query(&[("org", org), ("bucket", bucket)])
            .header("Authorization", format!("Token {token}"))
            .json(&serde_json::json!({
                "start": start,
                "stop": stop,
                "predicate": predicate,
            }))
            .send()
            .await
            .map_err(|error| crate::OrmerError::from_external("reqwest::Client::send", error))?;

        if response.status().is_success() {
            return Ok(BlockDeleteResult::default());
        }
        let status = response.status();
        let message = response
            .text()
            .await
            .unwrap_or_else(|_| "InfluxDB delete request failed".to_string());
        Err(crate::ormer_error!(
            "InfluxDB delete failed: {status}: {message}"
        ))
    }
}

/// 一个 InfluxQL 查询返回的 series（含 tags 与按列名组织的行）。
#[derive(Debug, Clone)]
pub(crate) struct InfluxSeries {
    pub tags: std::collections::BTreeMap<String, String>,
    pub rows: Vec<std::collections::BTreeMap<String, serde_json::Value>>,
}

fn parse_query_response(body: &str) -> crate::Result<Vec<InfluxSeries>> {
    let document: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| crate::ormer_error!("Invalid InfluxDB query response: {error}"))?;
    let mut series = Vec::new();
    if let Some(results) = document.get("results").and_then(serde_json::Value::as_array) {
        for result in results {
            if let Some(error) = result.get("error").and_then(serde_json::Value::as_str) {
                return Err(crate::ormer_error!("InfluxDB query error: {error}"));
            }
            if let Some(list) = result.get("series").and_then(serde_json::Value::as_array) {
                for entry in list {
                    let columns = entry
                        .get("columns")
                        .and_then(serde_json::Value::as_array)
                        .ok_or_else(|| crate::ormer_error!("Invalid InfluxDB series columns"))?
                        .iter()
                        .map(|value| {
                            value.as_str().unwrap_or_default().to_string()
                        })
                        .collect::<Vec<_>>();
                    let tags = entry
                        .get("tags")
                        .and_then(serde_json::Value::as_object)
                        .map(|tags| {
                            tags.iter()
                                .map(|(key, value)| {
                                    (
                                        key.clone(),
                                        value.as_str().unwrap_or_default().to_string(),
                                    )
                                })
                                .collect::<std::collections::BTreeMap<_, _>>()
                        })
                        .unwrap_or_default();
                    let rows = entry
                        .get("values")
                        .and_then(serde_json::Value::as_array)
                        .map(|values| {
                            values
                                .iter()
                                .map(|row| {
                                    let mut map = std::collections::BTreeMap::new();
                                    for (index, column) in columns.iter().enumerate() {
                                        map.insert(
                                            column.clone(),
                                            row.get(index).cloned().unwrap_or(serde_json::Value::Null),
                                        );
                                    }
                                    map
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    series.push(InfluxSeries { tags, rows });
                }
            }
        }
    }
    Ok(series)
}

fn parse_query_errors(body: &str) -> Vec<crate::OrmerError> {
    let Ok(document) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    if let Some(results) = document.get("results").and_then(serde_json::Value::as_array) {
        for result in results {
            if let Some(error) = result.get("error").and_then(serde_json::Value::as_str) {
                errors.push(crate::ormer_error!("InfluxDB statement error: {error}"));
            }
        }
    }
    errors
}

/// 把 series 行合并为按请求列取值的行集合；tags 作为补充列。
pub(crate) fn flatten_series_values(
    series: &[InfluxSeries],
    columns: Option<&[&str]>,
) -> crate::Result<Vec<Vec<Value>>> {
    let mut rows = Vec::new();
    for entry in series {
        for row in &entry.rows {
            match columns {
                Some(columns) => {
                    let mut values = Vec::with_capacity(columns.len());
                    for column in columns {
                        values.push(column_value_from_row(entry, row, column)?);
                    }
                    rows.push(values);
                }
                None => {
                    let mut columns: Vec<&String> = row
                        .keys()
                        .filter(|column| column.as_str() != "time")
                        .collect();
                    columns.sort();
                    if columns.len() == 1 {
                        let value = row.get(columns[0].as_str()).unwrap_or(&serde_json::Value::Null);
                        rows.push(vec![json_value_to_model_value(value)?]);
                        continue;
                    }
                    return Err(crate::ormer_error!(
                        "InfluxDB raw SQL requires a single-column result or a ViewModel/Model target"
                    ));
                }
            }
        }
    }
    Ok(rows)
}

fn column_value_from_row(
    entry: &InfluxSeries,
    row: &std::collections::BTreeMap<String, serde_json::Value>,
    column: &str,
) -> crate::Result<Value> {
    if let Some(value) = row.get(column) {
        return json_value_to_model_value(value);
    }
    if let Some(value) = entry.tags.get(column) {
        return Ok(Value::Text(value.clone()));
    }
    Err(crate::ormer_error!("Missing InfluxDB column: {column}"))
}

/// JSON 值 → 模型值（InfluxQL 返回 number/string/bool）。
fn json_value_to_model_value(value: &serde_json::Value) -> crate::Result<Value> {
    use serde_json::Value as Json;
    Ok(match value {
        Json::Null => Value::Null,
        Json::Bool(value) => Value::Boolean(*value),
        Json::Number(number) => {
            if let Some(value) = number.as_i64() {
                Value::Integer(value)
            } else if let Some(value) = number.as_u64() {
                Value::BigInt(value as i128)
            } else {
                Value::Real(number.as_f64().unwrap_or_default())
            }
        }
        Json::String(value) => Value::Text(value.clone()),
        Json::Array(_) | Json::Object(_) => {
            return Err(crate::ormer_error!(
                "InfluxDB does not return nested JSON values"
            ))
        }
    })
}

fn parse_json_u64(value: Option<&serde_json::Value>, field: &str) -> crate::Result<u64> {
    match value {
        Some(serde_json::Value::Number(value)) => value
            .as_u64()
            .ok_or_else(|| crate::ormer_error!("Invalid InfluxDB migration {field}")),
        Some(serde_json::Value::String(value)) => value
            .parse::<u64>()
            .map_err(|_| crate::ormer_error!("Invalid InfluxDB migration {field}")),
        Some(serde_json::Value::Null) => Err(crate::ormer_error!(
            "Invalid InfluxDB migration {field}"
        )),
        _ => Err(crate::ormer_error!("Invalid InfluxDB migration {field}")),
    }
}

pub(crate) fn quote_influx_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\\\""))
}

fn escape_tag_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(' ', "\\ ")
        .replace(',', "\\,")
        .replace('=', "\\=")
        .replace('\n', "\\n")
}

fn escape_field_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn quote_influxql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "\\'"))
}

/// 模型值 → InfluxQL 字面量（InfluxQL 不支持绑定参数，需内联）。
pub(crate) fn value_to_influxql_literal(value: &Value) -> crate::Result<String> {
    Ok(match value {
        Value::Null => "NULL".to_string(),
        Value::Boolean(value) => value.to_string(),
        Value::Integer(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::Duration(value) => value.as_micros().to_string(),
        Value::Real(value) => format_influx_float(*value),
        Value::Decimal(value) | Value::BigDecimal(value) => value.clone(),
        Value::Text(value) => quote_influxql_string(value),
        Value::Uuid(value) => quote_influxql_string(&value.to_string()),
        Value::Json(value) => quote_influxql_string(&value.to_string()),
        Value::DateTime(value) => {
            quote_influxql_string(&value.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
        }
        Value::Date(value) => quote_influxql_string(&value.to_string()),
        Value::Time(value) => quote_influxql_string(&value.to_string()),
        Value::Bytes(_) | Value::TextArray(_) | Value::IntegerArray(_) | Value::BigIntArray(_)
        | Value::NullableBigIntArray(_) => {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: crate::abstract_layer::DbType::InfluxDB,
                feature: "array/bytes query parameters",
            })
        }
    })
}

fn format_influx_float(value: f64) -> String {
    let rendered = value.to_string();
    if rendered.contains('.') || rendered.contains('e') || rendered.contains('E') {
        rendered
    } else {
        format!("{rendered}.0")
    }
}

/// std Duration → InfluxDB 时长（如 `30d`、`12h`、`90s`）。
pub(crate) fn format_influx_duration(duration: std::time::Duration) -> crate::Result<String> {
    let seconds = duration.as_secs();
    if seconds == 0 {
        return Err(crate::OrmerError::invalid_operation(
            "InfluxDB retention duration must be positive",
        ));
    }
    const WEEK: u64 = 7 * 86_400;
    const DAY: u64 = 86_400;
    const HOUR: u64 = 3_600;
    const MINUTE: u64 = 60;
    let (value, unit) = if seconds % WEEK == 0 {
        (seconds / WEEK, "w")
    } else if seconds % DAY == 0 {
        (seconds / DAY, "d")
    } else if seconds % HOUR == 0 {
        (seconds / HOUR, "h")
    } else if seconds % MINUTE == 0 {
        (seconds / MINUTE, "m")
    } else {
        (seconds, "s")
    };
    Ok(format!("{value}{unit}"))
}

pub(crate) fn retention_policy_name(table_name: &str) -> String {
    quote_influx_identifier(&format!("ormer_{table_name}"))
}

/// InfluxDB 模型约束：有且仅有一个时间类型 `#[primary]`（不支持 auto），
/// `#[index]` 字段必须为 String。不满足时报错。
pub(crate) fn validate_influx_model<T: Model>(db_type: crate::abstract_layer::DbType) -> crate::Result<()> {
    let schema = T::column_schema();
    if schema.iter().any(|column| column.is_auto_increment) {
        return Err(crate::OrmerError::UnsupportedFeature {
            backend: db_type,
            feature: "auto increment columns (InfluxDB writes are timestamp addressed)",
        });
    }
    let primaries = schema
        .iter()
        .filter(|column| column.is_primary)
        .collect::<Vec<_>>();
    let is_time_type = |rust_type: &str| {
        rust_type.starts_with("DateTime<")
            || rust_type.starts_with("chrono::DateTime<")
    };
    if primaries.len() != 1 || !primaries.first().is_some_and(|column| is_time_type(column.rust_type)) {
        return Err(crate::OrmerError::UnsupportedFeature {
            backend: db_type,
            feature:
                "InfluxDB models require exactly one time-typed #[primary] field as the timestamp",
        });
    }
    for column in schema.iter().filter(|column| column.is_indexed) {
        if column.rust_type != "String" {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature:
                    "InfluxDB #[index] fields (tags) must be declared as String (not Option<String>)",
            });
        }
    }
    Ok(())
}

/// 把模型集合渲染为 Line Protocol。
/// `#[index]` 字段构成 tag set，时间字段为时间戳，其余字段为 field。
pub(crate) fn render_line_protocol<T: Model>(models: &[&T]) -> crate::Result<String> {
    let db_type = crate::abstract_layer::DbType::InfluxDB;
    let measurement = T::table_name_for_db(db_type);
    let time_column = resolve_influx_time_key::<T>(db_type)?;
    let schema = T::column_schema();
    let mut lines = String::new();
    for model in models {
        let mut tags = schema
            .iter()
            .filter(|column| column.is_indexed)
            .map(|column| {
                let value = model
                    .column_value(column.name)
                    .ok_or_else(|| {
                        crate::ormer_error!(
                            "Missing InfluxDB tag value {} on model {}",
                            column.name,
                            T::TABLE_NAME
                        )
                    })?;
                let Value::Text(value) = value else {
                    return Err(crate::ormer_error!(
                        "InfluxDB tag {} must resolve to a String value",
                        column.name
                    ));
                };
                Ok(format!(
                    "{}={}",
                    escape_tag_value(column.name),
                    escape_tag_value(&value)
                ))
            })
            .collect::<crate::Result<Vec<_>>>()?;
        tags.sort();

        let timestamp = model
            .column_value(&time_column)
            .ok_or_else(|| {
                crate::ormer_error!(
                    "Missing InfluxDB timestamp value {time_column} on model {}",
                    T::TABLE_NAME
                )
            })?;
        let timestamp = value_to_nanoseconds(&timestamp)?;

        let mut fields = Vec::new();
        for column in schema.iter() {
            if column.is_indexed || column.name == time_column {
                continue;
            }
            let Some(value) = model.column_value(column.name) else {
                continue;
            };
            if matches!(value, Value::Null) {
                continue;
            }
            fields.push(format!(
                "{}={}",
                escape_tag_value(column.name),
                value_to_field_literal(&value)?
            ));
        }
        if fields.is_empty() {
            return Err(crate::ormer_error!(
                "InfluxDB point for measurement {measurement} has no field values"
            ));
        }

        lines.push_str(&escape_tag_value(measurement));
        if !tags.is_empty() {
            lines.push(',');
            lines.push_str(&tags.join(","));
        }
        lines.push(' ');
        lines.push_str(&fields.join(","));
        lines.push(' ');
        lines.push_str(&timestamp.to_string());
        lines.push('\n');
    }
    Ok(lines)
}

fn value_to_nanoseconds(value: &Value) -> crate::Result<i64> {
    match value {
        Value::DateTime(value) => value
            .timestamp_nanos_opt()
            .ok_or_else(|| crate::ormer_error!("InfluxDB timestamp is out of range")),
        Value::Date(value) => value
            .and_hms_opt(0, 0, 0)
            .and_then(|value| value.and_utc().timestamp_nanos_opt())
            .ok_or_else(|| crate::ormer_error!("InfluxDB timestamp is out of range")),
        Value::Integer(value) => Ok(i64::from(*value)),
        Value::BigInt(value) => i64::try_from(*value)
            .map_err(|_| crate::ormer_error!("InfluxDB timestamp is out of range")),
        Value::Text(value) => value
            .parse::<i64>()
            .map_err(|_| crate::ormer_error!("InfluxDB timestamp must be nanoseconds")),
        _ => Err(crate::ormer_error!(
            "InfluxDB timestamp field must be a time value"
        )),
    }
}

fn value_to_field_literal(value: &Value) -> crate::Result<String> {
    Ok(match value {
        Value::Integer(value) => format!("{value}i"),
        Value::BigInt(value) => format!("{value}i"),
        Value::Duration(value) => format!("{}i", value.as_micros()),
        Value::Real(value) => format_influx_float(*value),
        Value::Decimal(value) | Value::BigDecimal(value) => value.clone(),
        Value::Boolean(value) => value.to_string(),
        Value::Text(value) => format!("\"{}\"", escape_field_string(value)),
        Value::Uuid(value) => format!("\"{}\"", escape_field_string(&value.to_string())),
        Value::Json(value) => format!("\"{}\"", escape_field_string(&value.to_string())),
        Value::DateTime(value) => format!(
            "{}i",
            value
                .timestamp_nanos_opt()
                .ok_or_else(|| crate::ormer_error!("InfluxDB timestamp is out of range"))?
        ),
        Value::Date(value) => format!(
            "{}i",
            value
                .and_hms_opt(0, 0, 0)
                .and_then(|value| value.and_utc().timestamp_nanos_opt())
                .unwrap_or_default()
        ),
        Value::Time(value) => {
            use chrono::Timelike;
            format!(
                "{}i",
                value
                    .num_seconds_from_midnight()
                    .saturating_mul(1_000_000_000)
            )
        }
        Value::Bytes(_) | Value::TextArray(_) | Value::IntegerArray(_) | Value::BigIntArray(_)
        | Value::NullableBigIntArray(_) => {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: crate::abstract_layer::DbType::InfluxDB,
                feature: "array/bytes field values in Line Protocol",
            })
        }
        Value::Null => unreachable!("null fields are skipped by the caller"),
    })
}

pub struct BlockDeleteExecutor<'a, T: Model> {
    db: &'a Database,
    time_column: Option<String>,
    range: Option<BlockRange>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: Model> BlockDeleteExecutor<'a, T> {
    pub(crate) fn new(db: &'a Database) -> Self {
        Self {
            db,
            time_column: resolve_influx_time_key::<T>(crate::abstract_layer::DbType::InfluxDB)
                .ok(),
            range: None,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn with_range(mut self, range: BlockRange) -> Self {
        self.range = Some(range);
        self
    }

    pub fn to_sql(&self) -> crate::Result<crate::abstract_layer::common::SqlStatement> {
        Err(crate::OrmerError::UnsupportedFeature {
            backend: crate::abstract_layer::DbType::InfluxDB,
            feature: "block delete to_sql (the native backend uses the HTTP delete API)",
        })
    }

    pub async fn execute(self) -> crate::Result<BlockDeleteResult> {
        if self.time_column.is_none() {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: crate::abstract_layer::DbType::InfluxDB,
                feature: "block delete (declare #[hypertable(Duration)] or mark one DateTime field #[primary])",
            });
        }
        let Some(range) = self.range else {
            return Err(crate::OrmerError::invalid_operation(
                "block delete requires before(), between() or retain()",
            ));
        };
        self.db.delete_range::<T>(range, chrono::Utc::now()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_parses_org_bucket_token_and_strips_query() {
        let db = Database::connect(
            "http://localhost:8086?org=dev&bucket=metrics&token=my-token",
        )
        .unwrap();
        assert_eq!(db.url, "http://localhost:8086/");
        assert_eq!(db.database_name(), "metrics");
        assert!(matches!(
            db.mode,
            InfluxMode::V2 { ref org, ref bucket, ref token }
                if org == "dev" && bucket == "metrics" && token == "my-token"
        ));
    }

    #[test]
    fn connect_parses_v1_database_user_password() {
        let db = Database::connect(
            "http://localhost:8086?database=telegraf&user=admin&password=secret",
        )
        .unwrap();
        assert_eq!(db.url, "http://localhost:8086/");
        assert!(matches!(
            db.mode,
            InfluxMode::V1 { ref database, ref user, ref password }
                if database == "telegraf" && user == "admin" && password == "secret"
        ));
        assert_eq!(db.database_name(), "telegraf");
        assert!(db.auth_header().is_none());
        assert_eq!(db.write_path(), "http://localhost:8086/write");
    }

    #[test]
    fn connect_requires_org_bucket_and_token() {
        assert!(Database::connect("http://localhost:8086").is_err());
        assert!(Database::connect("http://localhost:8086?bucket=metrics&token=t").is_err());
        assert!(Database::connect("http://localhost:8086?org=dev&token=t").is_err());
        assert!(Database::connect("http://localhost:8086?org=dev&bucket=metrics").is_err());
        // 1.x 允许只给 database（无认证的本地实例）
        assert!(Database::connect("http://localhost:8086?database=telegraf").is_ok());
        assert!(
            Database::connect("http://localhost:8086?org=&bucket=metrics&token=t").is_err(),
            "empty org must be rejected"
        );
        // 1.x 必须提供 database
        assert!(Database::connect("http://localhost:8086?user=u&password=p").is_err());
    }

    #[test]
    fn connect_rejects_invalid_url() {
        assert!(Database::connect("not a url").is_err());
    }

    #[test]
    fn inline_sql_substitutes_placeholders() {
        let sql = "SELECT * FROM cpu WHERE host = ? AND usage > ?";
        let params = vec![Value::Text("server-1".to_string()), Value::Real(62.5)];
        assert_eq!(
            Database::inline_sql(sql, &params).unwrap(),
            "SELECT * FROM cpu WHERE host = 'server-1' AND usage > 62.5"
        );
        assert!(Database::inline_sql(sql, &[Value::Integer(1)]).is_err());
        assert!(Database::inline_sql("SELECT 1", &[Value::Integer(1)]).is_err());
        // 字符串字面量中的 ? 不替换
        assert_eq!(
            Database::inline_sql("SELECT 'a?b'", &[]).unwrap(),
            "SELECT 'a?b'"
        );
    }

    #[test]
    fn value_literals_render_influxql_types() {
        assert_eq!(
            value_to_influxql_literal(&Value::Text("it's".to_string())).unwrap(),
            "'it\\'s'"
        );
        assert_eq!(
            value_to_influxql_literal(&Value::Real(62.0)).unwrap(),
            "62.0"
        );
        assert_eq!(
            value_to_influxql_literal(&Value::Boolean(true)).unwrap(),
            "true"
        );
    }

    #[test]
    fn durations_render_as_influx_units() {
        assert_eq!(
            format_influx_duration(std::time::Duration::from_secs(30 * 86_400)).unwrap(),
            "30d"
        );
        assert_eq!(
            format_influx_duration(std::time::Duration::from_secs(7 * 86_400)).unwrap(),
            "1w"
        );
        assert_eq!(
            format_influx_duration(std::time::Duration::from_secs(3_600)).unwrap(),
            "1h"
        );
        assert_eq!(
            format_influx_duration(std::time::Duration::from_secs(90)).unwrap(),
            "90s"
        );
        assert!(format_influx_duration(std::time::Duration::ZERO).is_err());
    }

    #[test]
    fn query_response_parses_series_and_tags() {
        let body = r#"{"results":[{"statement_id":0,"series":[
            {"name":"cpu_usage","tags":{"host":"server-1"},"columns":["time","count"],
             "values":[["2026-09-09T00:00:00Z",3]]}
        ]}]}"#;
        let series = parse_query_response(body).unwrap();
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].tags.get("host").map(String::as_str), Some("server-1"));
        assert_eq!(
            series[0].rows[0].get("count"),
            Some(&serde_json::json!(3))
        );
        let values =
            flatten_series_values(&series, Some(&["count"])).unwrap();
        assert!(matches!(values.as_slice(), [row] if matches!(row.as_slice(), [Value::Integer(3)])));
        let single = flatten_series_values(&series, None).unwrap();
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn query_response_reports_embedded_errors() {
        let body = r#"{"results":[{"error":"database not found"}]}"#;
        assert!(parse_query_response(body).is_err());
        assert!(!parse_query_errors(body).is_empty());
    }

    #[derive(Debug, ormer::Model, Clone)]
    #[table = "cpu_usage"]
    struct CpuUsage {
        #[primary]
        time: chrono::DateTime<chrono::Utc>,
        #[index]
        host: String,
        usage: f64,
    }

    #[derive(Debug, ormer::Model, Clone)]
    #[table = "cpu_daily"]
    struct CpuDaily {
        #[primary]
        id: i64,
        #[hypertable(std::time::Duration::from_secs(86_400))]
        time: chrono::DateTime<chrono::Utc>,
        usage: f64,
    }

    #[derive(Debug, ormer::Model, Clone)]
    #[table = "no_time_key"]
    struct NoTimeKey {
        #[primary]
        id: i64,
        usage: f64,
    }

    #[test]
    fn time_key_prefers_hypertable_then_falls_back_to_primary_datetime() {
        let backend = crate::abstract_layer::DbType::InfluxDB;
        // #[hypertable] 声明优先
        assert_eq!(
            resolve_influx_time_key::<CpuDaily>(backend).unwrap(),
            "time"
        );
        // 无 hypertable 时回退为唯一的 DateTime #[primary] 字段
        assert_eq!(resolve_influx_time_key::<CpuUsage>(backend).unwrap(), "time");
        // 两者都没有时显式报错
        let error = resolve_influx_time_key::<NoTimeKey>(backend).unwrap_err();
        assert!(matches!(
            error,
            crate::OrmerError::UnsupportedFeature { .. }
        ));
    }

    #[test]
    fn model_validation_requires_time_primary_and_string_tags() {
        let backend = crate::abstract_layer::DbType::InfluxDB;
        assert!(validate_influx_model::<CpuUsage>(backend).is_ok());
        // 非时间主键不允许
        assert!(validate_influx_model::<NoTimeKey>(backend).is_err());

        #[derive(Debug, ormer::Model, Clone)]
        #[table = "tagged"]
        struct Tagged {
            #[primary]
            time: chrono::DateTime<chrono::Utc>,
            #[index]
            host: String,
            #[index]
            bad: i64,
        }
        assert!(validate_influx_model::<Tagged>(backend).is_err());

        #[derive(Debug, ormer::Model, Clone)]
        #[table = "auto_pk"]
        struct AutoPk {
            #[primary(auto = true)]
            id: i64,
        }
        assert!(validate_influx_model::<AutoPk>(backend).is_err());
    }

    #[test]
    fn line_protocol_renders_tags_fields_and_timestamp() {
        let points = vec![
            CpuUsage {
                time: chrono::DateTime::from_timestamp(1_700_000_000, 123).unwrap(),
                host: "server-1".to_string(),
                usage: 62.5,
            },
            CpuUsage {
                time: chrono::DateTime::from_timestamp(1_700_000_001, 0).unwrap(),
                host: "server,2".to_string(),
                usage: 70.0,
            },
        ];
        let refs = points.iter().collect::<Vec<_>>();
        let lines = render_line_protocol(&refs).unwrap();
        let mut lines = lines.lines();
        let first = lines.next().unwrap();
        assert_eq!(
            first,
            "cpu_usage,host=server-1 usage=62.5 1700000000000000123"
        );
        let second = lines.next().unwrap();
        assert_eq!(
            second,
            "cpu_usage,host=server\\,2 usage=70.0 1700000001000000000"
        );
        // 即使没有数据点，时间键解析失败也要报错
        assert!(render_line_protocol::<NoTimeKey>(&[]).is_err());
    }

    #[test]
    fn executor_to_sql_reports_http_only_protocol() {
        // InfluxDB 无 SQL 传输层，块删除走 HTTP /api/v2/delete，
        // to_sql 协议在此显式报不支持而不是渲染伪 SQL。
        let error = BlockDeleteExecutor::<CpuUsage>::new(&Database {
            http: reqwest::Client::new(),
            url: "http://localhost:8086/".to_string(),
            mode: InfluxMode::V2 {
                org: "dev".to_string(),
                bucket: "metrics".to_string(),
                token: "t".to_string(),
            },
        })
        .to_sql()
        .unwrap_err();
        match error {
            crate::OrmerError::UnsupportedFeature { feature, .. } => {
                assert!(feature.contains("to_sql"), "unexpected: {feature}");
                assert!(feature.contains("HTTP"), "unexpected: {feature}");
            }
            other => panic!("expected UnsupportedFeature, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn executor_execute_requires_range() {
        let error = BlockDeleteExecutor::<CpuUsage>::new(&Database {
            http: reqwest::Client::new(),
            url: "http://localhost:8086/".to_string(),
            mode: InfluxMode::V2 {
                org: "dev".to_string(),
                bucket: "metrics".to_string(),
                token: "t".to_string(),
            },
        })
        .execute()
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            crate::OrmerError::InvalidOperation { .. }
        ));
    }

    #[test]
    fn delete_request_uses_raw_time_bounds_not_aligned_blocks() {
        // InfluxDB 由服务端按 shard 组织，不做块对齐：
        // before 的 cutoff 原样作为 stop 下发。
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::hours(3);
        let bounds = time_delete_bounds(BlockRange::Before { cutoff }, now).unwrap();
        assert_eq!(bounds, Some((None, cutoff)));
        // between 保留原始起止
        let start = now - chrono::Duration::hours(8);
        let bounds =
            time_delete_bounds(BlockRange::Between { start, end: cutoff }, now).unwrap();
        assert_eq!(bounds, Some((Some(start), cutoff)));
        // between 的 start >= end 报参数错误
        assert!(time_delete_bounds(BlockRange::Between { start: cutoff, end: start }, now).is_err());
    }
}
