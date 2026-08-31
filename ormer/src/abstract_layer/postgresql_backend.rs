use super::common::common_helpers;
use crate::abstract_layer::DbType;
use crate::abstract_layer::common::{SingleSqlStatement, SqlExecutor, SqlStatement};
use crate::db_first::{
    DbFirstColumn, DbFirstForeignKey, DbFirstIndex, DbFirstIndexColumn, DbFirstTable,
};
use crate::hooks::{HookContext, HookOperation};
use crate::migration::{SchemaColumn, schema_column_with_compression};
use crate::model::{
    DbBackendTypeMapper, DurationToInterval, Model, Row, Value, WritableModel,
    quote_qualified_identifier, split_schema_table_name,
};
use crate::query::builder::{
    FourTableSelect, GroupedSelect, InnerJoinedSelect, LeftJoinedSelect, MultiTableSelect,
    RelatedSelect, RightJoinedSelect, Select, WhereExpr,
};
use crate::query::expr::SqlExpr;
use crate::query::filter::{
    FilterExpr, OrderBy, infer_filter_value_rust_type, infer_model_value_rust_type,
};
use crate::query::insert::{
    InsertAssignment, InsertConflict, IntoInsertAssignment, IntoInsertDefaultColumn,
};
use crate::query::update::UpdateAssignment;
use crate::query::update::{UpdateExpr, UpdateValue};
use crate::raw_sql::IntoRawSql;
use crate::utils::{FutureTraceExt, ResultTraceExt};
use crate::{
    impl_backend_executor_methods, impl_backend_four_table_executor_methods_with_lifetime,
    impl_backend_join_executor_methods_with_lifetime,
    impl_backend_multi_table_executor_methods_with_lifetime,
    impl_backend_related_executor_methods_with_lifetime, impl_insert_conflict_methods,
};
use bytes::{BufMut, Bytes, BytesMut};
use futures::SinkExt;
use postgres_types::{FromSql, FromSqlOwned, IsNull, ToSql, Type as PgType};
use std::collections::HashMap;
use std::marker::PhantomData;
use tokio_postgres::NoTls;
use tokio_postgres::types::Type;

type ModelUpdateBatch = common_helpers::ModelUpdateBatch;
type UpdateSqlBatch = Vec<(common_helpers::ModelSqlStatement, Vec<&'static str>)>;
type PostgreSQLParam = Box<dyn ToSql + Sync + Send>;

fn pg_param_refs(params: &[PostgreSQLParam]) -> Vec<&(dyn ToSql + Sync)> {
    params
        .iter()
        .map(|param| param.as_ref() as &(dyn ToSql + Sync))
        .collect()
}

async fn pg_query_with_types(
    client: &tokio_postgres::Client,
    sql: &str,
    params: &[Value],
    rust_types: &[&str],
) -> crate::Result<Vec<tokio_postgres::Row>> {
    let pg_params = values_to_params_with_types(params, rust_types)?;
    let param_refs = pg_param_refs(&pg_params);
    traced_pg_query(client, sql, params, &param_refs).await
}

async fn pg_execute_with_types(
    client: &tokio_postgres::Client,
    sql: &str,
    params: &[Value],
    rust_types: &[&str],
) -> crate::Result<u64> {
    let pg_params = values_to_params_with_types(params, rust_types)?;
    let param_refs = pg_param_refs(&pg_params);
    traced_pg_execute(client, sql, params, &param_refs).await
}

async fn pg_query_for_query(
    client: &tokio_postgres::Client,
    sql: &str,
    params: &[Value],
    rust_types: &[&str],
) -> crate::Result<Vec<tokio_postgres::Row>> {
    let pg_params = values_to_params_for_query(params, rust_types)?;
    let param_refs = pg_param_refs(&pg_params);
    traced_pg_query(client, sql, params, &param_refs).await
}

async fn pg_query_untyped(
    client: &tokio_postgres::Client,
    sql: &str,
    params: &[Value],
) -> crate::Result<Vec<tokio_postgres::Row>> {
    let pg_params = values_to_params(params)?;
    let param_refs = pg_param_refs(&pg_params);
    traced_pg_query(client, sql, params, &param_refs).await
}

async fn pg_execute_untyped(
    client: &tokio_postgres::Client,
    sql: &str,
    params: &[Value],
) -> crate::Result<u64> {
    let pg_params = values_to_params(params)?;
    let param_refs = pg_param_refs(&pg_params);
    traced_pg_execute(client, sql, params, &param_refs).await
}

async fn traced_pg_query(
    client: &tokio_postgres::Client,
    sql: &str,
    trace_params: &[Value],
    params: &[&(dyn ToSql + Sync)],
) -> crate::Result<Vec<tokio_postgres::Row>> {
    let trace = crate::sql_trace::start_sql_trace(sql, trace_params);
    match client.query(trace.sql(), params).await {
        Ok(rows) => {
            trace.finish_ok();
            Ok(rows)
        }
        Err(error) => Err(trace.finish_external_error("tokio_postgres::Client::query", error)),
    }
}

async fn traced_pg_execute(
    client: &tokio_postgres::Client,
    sql: &str,
    trace_params: &[Value],
    params: &[&(dyn ToSql + Sync)],
) -> crate::Result<u64> {
    let trace = crate::sql_trace::start_sql_trace(sql, trace_params);
    match client.execute(trace.sql(), params).await {
        Ok(rows) => {
            trace.finish_ok();
            Ok(rows)
        }
        Err(error) => Err(trace.finish_external_error("tokio_postgres::Client::execute", error)),
    }
}

async fn traced_pg_execute_empty(client: &tokio_postgres::Client, sql: &str) -> crate::Result<u64> {
    traced_pg_execute(client, sql, &[], &[]).await
}

fn append_postgresql_upsert_clause<T: Model>(sql: &mut String, columns: &[&str]) {
    let primary_key_columns = T::primary_key_columns();
    let quoted_primary_keys =
        common_helpers::quote_column_list(DbType::PostgreSQL, primary_key_columns);

    sql.push_str(&format!(
        " ON CONFLICT ({quoted_primary_keys}) DO UPDATE SET "
    ));

    let mut first = true;
    for col_name in columns.iter() {
        if primary_key_columns.contains(col_name) {
            continue;
        }
        if !first {
            sql.push_str(", ");
        }
        sql.push_str(&common_helpers::quote_postgres_excluded_assignment(
            DbType::PostgreSQL,
            col_name,
        ));
        first = false;
    }
}

fn pg_column_rust_type<T: Model>(column: &str) -> Option<&'static str> {
    let column = column.rsplit('.').next().unwrap_or(column);
    T::COLUMN_SCHEMA
        .iter()
        .find(|schema| schema.name == column)
        .map(|schema| schema.data_type.unwrap_or(schema.rust_type))
}

fn pg_collect_update_value_rust_types<T: Model>(
    column: &str,
    value: &UpdateValue,
    rust_types: &mut Vec<&'static str>,
) {
    let column_rust_type = pg_column_rust_type::<T>(column).unwrap_or("String");
    match value {
        UpdateValue::Literal(value) => {
            rust_types.push(if matches!(value, crate::model::Value::Null) {
                column_rust_type
            } else {
                infer_model_value_rust_type(value)
            })
        }
        UpdateValue::Expr(expr) => {
            pg_collect_update_expr_rust_types::<T>(column_rust_type, expr, rust_types)
        }
    }
}

fn pg_collect_update_expr_rust_types<T: Model>(
    column_rust_type: &'static str,
    expr: &UpdateExpr,
    rust_types: &mut Vec<&'static str>,
) {
    match expr {
        UpdateExpr::Column(_) | UpdateExpr::IncomingColumn(_) => {}
        UpdateExpr::Value(value) => {
            rust_types.push(if matches!(value, crate::model::Value::Null) {
                column_rust_type
            } else {
                infer_model_value_rust_type(value)
            })
        }
        UpdateExpr::Binary { left, right, .. } => {
            pg_collect_update_expr_rust_types::<T>(column_rust_type, left, rust_types);
            pg_collect_update_expr_rust_types::<T>(column_rust_type, right, rust_types);
        }
        UpdateExpr::Sql(expr) => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
        }
    }
}

fn pg_collect_conflict_rust_types<T: Model>(
    conflict: &InsertConflict,
    rust_types: &mut Vec<&'static str>,
) {
    if let Some(target_filter) = &conflict.target_filter {
        pg_collect_filter_param_rust_types::<T>(target_filter, rust_types);
    }
    for assignment in &conflict.assignments {
        pg_collect_update_value_rust_types::<T>(&assignment.column, &assignment.value, rust_types);
    }
    if let Some(filter) = &conflict.update_filter {
        pg_collect_filter_param_rust_types::<T>(filter, rust_types);
    }
}

pub(crate) fn pg_insert_param_rust_types<T: Model>(
    row_count: usize,
    conflict: Option<&InsertConflict>,
) -> Vec<&'static str> {
    let insert_types: Vec<&str> = T::COLUMN_SCHEMA
        .iter()
        .filter(|col| !col.is_auto_increment)
        .map(|col| col.data_type.unwrap_or(col.rust_type))
        .collect();
    let mut rust_types = Vec::with_capacity(row_count * insert_types.len());
    for _ in 0..row_count {
        rust_types.extend(insert_types.iter().copied());
    }
    if let Some(conflict) = conflict {
        pg_collect_conflict_rust_types::<T>(conflict, &mut rust_types);
    }
    rust_types
}

const POSTGRES_COPY_MIN_ROWS: usize = 1024;

fn pg_copy_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn pg_copy_value(value: &Value) -> Option<String> {
    match value {
        Value::Integer(value) => Some(value.to_string()),
        Value::BigInt(value) => Some(value.to_string()),
        Value::Duration(value) => Some(value.to_interval_string()),
        Value::Text(value) => Some(pg_copy_escape(value)),
        Value::Real(value) => Some(value.to_string()),
        Value::Decimal(value) => Some(value.to_string()),
        Value::BigDecimal(value) => Some(value.to_string()),
        Value::Boolean(value) => Some(value.to_string()),
        Value::DateTime(value) => Some(value.to_string()),
        Value::Date(value) => Some(value.to_string()),
        Value::Time(value) => Some(value.to_string()),
        Value::Json(value) => Some(pg_copy_escape(&value.to_string())),
        Value::Uuid(value) => Some(value.to_string()),
        Value::Null => Some("\\N".to_string()),
        Value::Bytes(_)
        | Value::TextArray(_)
        | Value::IntegerArray(_)
        | Value::BigIntArray(_)
        | Value::NullableBigIntArray(_) => None,
    }
}

fn pg_copy_insert_payload<T: Model>(models: &[&T]) -> Option<String> {
    let mut payload = String::new();
    for model in models {
        for (index, value) in model.insert_values().iter().enumerate() {
            if index > 0 {
                payload.push('\t');
            }
            payload.push_str(&pg_copy_value(value)?);
        }
        payload.push('\n');
    }
    Some(payload)
}

fn pg_copy_insert_statement<T: Model>(models: &[&T]) -> crate::Result<String> {
    let columns = T::insert_columns();
    let table_name = common_helpers::routed_insert_table_name::<T>(DbType::PostgreSQL, models)?;
    Ok(format!(
        "COPY {} ({}) FROM STDIN",
        quote_qualified_identifier(DbType::PostgreSQL, &table_name),
        common_helpers::quote_column_list(DbType::PostgreSQL, &columns)
    ))
}

fn pg_should_use_copy_insert<T: Model>(models: &[&T], conflict: Option<&InsertConflict>) -> bool {
    models.len() >= POSTGRES_COPY_MIN_ROWS
        && common_helpers::auto_increment_column::<T>().is_none()
        && !conflict.is_some_and(InsertConflict::is_configured)
        && !T::insert_columns().is_empty()
}

async fn pg_try_copy_insert<T: Model>(
    client: &tokio_postgres::Client,
    models: &[&T],
) -> crate::Result<bool> {
    let Some(payload) = pg_copy_insert_payload(models) else {
        return Ok(false);
    };
    let copy_sql = pg_copy_insert_statement::<T>(models)?;
    let sink = client.copy_in(&copy_sql).await?;
    let mut sink = std::pin::pin!(sink);
    sink.send(Bytes::from(payload)).await?;
    sink.finish().await?;
    Ok(true)
}

/// PostgreSQL 类型映射器
pub struct PostgreSQLTypeMapper;

#[derive(Debug, Clone, Copy)]
struct PgInterval {
    microseconds: i64,
    days: i32,
    months: i32,
}

impl<'a> FromSql<'a> for PgInterval {
    fn from_sql(_: &PgType, raw: &[u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        use byteorder::{BigEndian, ReadBytesExt};
        let mut raw = raw;
        let microseconds = raw.read_i64::<BigEndian>()?;
        let days = raw.read_i32::<BigEndian>()?;
        let months = raw.read_i32::<BigEndian>()?;
        Ok(Self {
            microseconds,
            days,
            months,
        })
    }

    postgres_types::accepts!(INTERVAL);
}

impl ToSql for PgInterval {
    fn to_sql(
        &self,
        _: &PgType,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        out.put_i64(self.microseconds);
        out.put_i32(self.days);
        out.put_i32(self.months);
        Ok(IsNull::No)
    }

    postgres_types::accepts!(INTERVAL);
    postgres_types::to_sql_checked!();
}

#[derive(Debug, Clone)]
struct PgTextParam(String);

impl From<String> for PgTextParam {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl ToSql for PgTextParam {
    fn to_sql(
        &self,
        ty: &PgType,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        <&str as ToSql>::to_sql(&self.0.as_str(), ty, out)
    }

    fn accepts(ty: &PgType) -> bool {
        matches!(
            *ty,
            PgType::VARCHAR | PgType::TEXT | PgType::BPCHAR | PgType::NAME | PgType::UNKNOWN
        ) || matches!(ty.kind(), postgres_types::Kind::Enum(_))
            || matches!(ty.name(), "citext" | "ltree" | "lquery" | "ltxtquery")
    }

    fn encode_format(&self, _ty: &PgType) -> postgres_types::Format {
        postgres_types::Format::Text
    }

    postgres_types::to_sql_checked!();
}

#[derive(Debug, Clone)]
struct PgNumericTextParam(String);

impl ToSql for PgNumericTextParam {
    fn to_sql(
        &self,
        _: &PgType,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        out.put_slice(self.0.as_bytes());
        Ok(IsNull::No)
    }

    fn accepts(ty: &PgType) -> bool {
        matches!(*ty, PgType::NUMERIC | PgType::UNKNOWN)
    }

    fn encode_format(&self, _ty: &PgType) -> postgres_types::Format {
        postgres_types::Format::Text
    }

    postgres_types::to_sql_checked!();
}

#[derive(Debug, Clone)]
struct PgMaybeNumericTextParam(Option<String>);

impl ToSql for PgMaybeNumericTextParam {
    fn to_sql(
        &self,
        ty: &PgType,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match &self.0 {
            Some(value) => PgNumericTextParam(value.clone()).to_sql(ty, out),
            None => Ok(IsNull::Yes),
        }
    }

    fn accepts(ty: &PgType) -> bool {
        PgNumericTextParam::accepts(ty)
    }

    fn encode_format(&self, _ty: &PgType) -> postgres_types::Format {
        postgres_types::Format::Text
    }

    postgres_types::to_sql_checked!();
}

#[derive(Debug, Clone)]
struct PgNumericText(String);

impl<'a> FromSql<'a> for PgNumericText {
    fn from_sql(
        ty: &PgType,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        if *ty != PgType::NUMERIC {
            return Err("expected NUMERIC".into());
        }
        Ok(Self(postgres_numeric_to_string(raw)?))
    }

    fn accepts(ty: &PgType) -> bool {
        *ty == PgType::NUMERIC
    }
}

#[derive(Debug, Clone)]
struct PgMaybeTextParam(Option<String>);

impl ToSql for PgMaybeTextParam {
    fn to_sql(
        &self,
        ty: &PgType,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match &self.0 {
            Some(value) => <&str as ToSql>::to_sql(&value.as_str(), ty, out),
            None => Ok(IsNull::Yes),
        }
    }

    fn accepts(ty: &PgType) -> bool {
        PgTextParam::accepts(ty)
    }

    fn encode_format(&self, _ty: &PgType) -> postgres_types::Format {
        postgres_types::Format::Text
    }

    postgres_types::to_sql_checked!();
}

#[derive(Debug, Clone, Copy)]
struct PgDateTimeParam(chrono::DateTime<chrono::Utc>);

impl ToSql for PgDateTimeParam {
    fn to_sql(
        &self,
        ty: &PgType,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        if *ty == PgType::TIMESTAMP {
            self.0.naive_utc().to_sql(ty, out)
        } else {
            self.0.to_sql(ty, out)
        }
    }

    fn accepts(ty: &PgType) -> bool {
        matches!(*ty, PgType::TIMESTAMP | PgType::TIMESTAMPTZ)
    }

    postgres_types::to_sql_checked!();
}

#[derive(Debug, Clone, Copy)]
struct PgMaybeDateTimeParam(Option<chrono::DateTime<chrono::Utc>>);

impl ToSql for PgMaybeDateTimeParam {
    fn to_sql(
        &self,
        ty: &PgType,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self.0 {
            Some(value) => PgDateTimeParam(value).to_sql(ty, out),
            None => Ok(IsNull::Yes),
        }
    }

    fn accepts(ty: &PgType) -> bool {
        PgDateTimeParam::accepts(ty)
    }

    postgres_types::to_sql_checked!();
}

#[derive(Debug, Clone)]
struct PgEnumText(String);

impl<'a> FromSql<'a> for PgEnumText {
    fn from_sql(
        ty: &PgType,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(<&str as FromSql>::from_sql(ty, raw)?.to_string()))
    }

    fn accepts(ty: &PgType) -> bool {
        matches!(ty.kind(), postgres_types::Kind::Enum(_))
            || matches!(
                *ty,
                PgType::VARCHAR | PgType::TEXT | PgType::BPCHAR | PgType::NAME | PgType::UNKNOWN
            )
    }
}

fn to_postgres_interval(duration: std::time::Duration) -> PgInterval {
    let micros_u128 = duration.as_micros();
    let micros = micros_u128.min(i64::MAX as u128) as i64;
    let days = micros / 86_400_000_000;
    let micros_after_days = micros - days * 86_400_000_000;
    PgInterval {
        microseconds: micros_after_days,
        days: days as i32,
        months: 0,
    }
}

fn from_postgres_interval(interval: PgInterval) -> std::time::Duration {
    let month_micros = i128::from(interval.months) * 30 * 86_400_000_000i128;
    let day_micros = i128::from(interval.days) * 86_400_000_000i128;
    let total_micros = month_micros + day_micros + i128::from(interval.microseconds);
    if total_micros <= 0 {
        std::time::Duration::ZERO
    } else {
        std::time::Duration::from_micros(total_micros.min(u64::MAX as i128) as u64)
    }
}

fn postgres_numeric_to_string(
    raw: &[u8],
) -> Result<String, Box<dyn std::error::Error + Sync + Send>> {
    use byteorder::{BigEndian, ReadBytesExt};
    const NUMERIC_NEG: u16 = 0x4000;
    const NUMERIC_NAN: u16 = 0xC000;
    const NUMERIC_PINF: u16 = 0xD000;
    const NUMERIC_NINF: u16 = 0xF000;

    let mut cursor = raw;
    let digit_count = cursor.read_u16::<BigEndian>()? as usize;
    let weight = cursor.read_i16::<BigEndian>()?;
    let sign = cursor.read_u16::<BigEndian>()?;
    let display_scale = cursor.read_u16::<BigEndian>()? as usize;

    return match sign {
        NUMERIC_NAN => Ok("NaN".to_string()),
        NUMERIC_PINF => Ok("Infinity".to_string()),
        NUMERIC_NINF => Ok("-Infinity".to_string()),
        NUMERIC_NEG | 0 => {
            let mut groups = Vec::with_capacity(digit_count);
            for _ in 0..digit_count {
                groups.push(cursor.read_u16::<BigEndian>()?);
            }

            let integer_groups = (weight + 1).max(0) as usize;
            let mut digits = String::new();
            if integer_groups == 0 {
                digits.push('0');
            } else {
                for i in 0..integer_groups {
                    let group = groups.get(i).copied().unwrap_or(0);
                    if i == 0 {
                        digits.push_str(&group.to_string());
                    } else {
                        digits.push_str(&format!("{group:04}"));
                    }
                }
            }

            if display_scale > 0 {
                digits.push('.');
                let fraction_groups = display_scale.div_ceil(4);
                for i in integer_groups..integer_groups + fraction_groups {
                    let group = groups.get(i).copied().unwrap_or(0);
                    digits.push_str(&format!("{group:04}"));
                }
                digits.truncate(digits.len() - fraction_groups * 4 + display_scale);
            }

            if sign == NUMERIC_NEG && digits != "0" {
                digits.insert(0, '-');
            }
            Ok(digits)
        }
        _ => Err("invalid NUMERIC sign".into()),
    };
}

fn pg_datetime_value_from_row(
    row: &tokio_postgres::Row,
    idx: usize,
) -> crate::Result<Option<chrono::DateTime<chrono::Utc>>> {
    if let Ok(value) = row.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(idx) {
        return Ok(value);
    }
    if let Ok(value) = row.try_get::<_, Option<chrono::NaiveDateTime>>(idx) {
        return Ok(
            value.map(|value| chrono::DateTime::from_naive_utc_and_offset(value, chrono::Utc))
        );
    }
    Err(crate::ormer_error!(
        "Failed to parse column at index {idx} (expected PostgreSQL timestamp type)"
    ))
}

fn pg_date_value_from_row(
    row: &tokio_postgres::Row,
    idx: usize,
) -> crate::Result<Option<chrono::NaiveDate>> {
    row.try_get(idx).map_err(|err| {
        crate::ormer_error!(
            "Failed to parse column at index {idx} (expected PostgreSQL date type): {err}"
        )
    })
}

fn pg_time_value_from_row(
    row: &tokio_postgres::Row,
    idx: usize,
) -> crate::Result<Option<chrono::NaiveTime>> {
    row.try_get(idx).map_err(|err| {
        crate::ormer_error!(
            "Failed to parse column at index {idx} (expected PostgreSQL time type): {err}"
        )
    })
}

fn pg_try_get<T: FromSqlOwned>(
    row: &tokio_postgres::Row,
    idx: usize,
    expected_type: &str,
) -> crate::Result<Option<T>> {
    let actual_type = row
        .columns()
        .get(idx)
        .map(|column| column.type_().name())
        .unwrap_or("<out of range>");
    row.try_get(idx).map_err(|err| {
        crate::ormer_error!(
            "Failed to parse column at index {idx} (expected {expected_type}, actual PostgreSQL type {actual_type}): {err}"
        )
    })
}

fn pg_model_column_rust_type<T: Model>(column: &str) -> Option<&'static str> {
    let column = column.rsplit('.').next().unwrap_or(column);
    T::COLUMN_SCHEMA
        .iter()
        .find(|schema| schema.name == column)
        .map(|schema| schema.data_type.unwrap_or(schema.rust_type))
}

fn pg_collect_update_expr_param_rust_types<T: Model>(
    expr: &UpdateExpr,
    column: &str,
    rust_types: &mut Vec<&'static str>,
) {
    match expr {
        UpdateExpr::Column(_) | UpdateExpr::IncomingColumn(_) => {}
        UpdateExpr::Value(value) => rust_types.push(
            pg_model_column_rust_type::<T>(column)
                .unwrap_or_else(|| infer_model_value_rust_type(value)),
        ),
        UpdateExpr::Binary { left, right, .. } => {
            pg_collect_update_expr_param_rust_types::<T>(left, column, rust_types);
            pg_collect_update_expr_param_rust_types::<T>(right, column, rust_types);
        }
        UpdateExpr::Sql(expr) => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
        }
    }
}

fn pg_collect_update_assignment_param_rust_types<T: Model>(
    assignment: &UpdateAssignment,
    rust_types: &mut Vec<&'static str>,
) {
    match &assignment.value {
        UpdateValue::Literal(value) => rust_types.push(
            pg_model_column_rust_type::<T>(&assignment.column)
                .unwrap_or_else(|| infer_model_value_rust_type(value)),
        ),
        UpdateValue::Expr(expr) => {
            pg_collect_update_expr_param_rust_types::<T>(expr, &assignment.column, rust_types);
        }
    }
}

fn pg_model_statement_param_rust_types<T: Model>(
    statement: &common_helpers::ModelSqlStatement,
) -> Option<Vec<&'static str>> {
    statement.param_columns.as_ref().map(|columns| {
        columns
            .iter()
            .zip(&statement.params)
            .map(|(column, value)| {
                pg_model_column_rust_type::<T>(column)
                    .unwrap_or_else(|| infer_model_value_rust_type(value))
            })
            .collect()
    })
}

fn is_vec_i32_type(rust_type: &str) -> bool {
    matches!(
        rust_type,
        "Vec<i32>" | "std::vec::Vec<i32>" | "alloc::vec::Vec<i32>"
    )
}

fn is_vec_i64_type(rust_type: &str) -> bool {
    matches!(
        rust_type,
        "Vec<i64>" | "std::vec::Vec<i64>" | "alloc::vec::Vec<i64>"
    )
}

fn is_vec_option_i64_type(rust_type: &str) -> bool {
    matches!(
        rust_type,
        "Vec<Option<i64>>" | "std::vec::Vec<Option<i64>>" | "alloc::vec::Vec<Option<i64>>"
    )
}

fn is_vec_string_type(rust_type: &str) -> bool {
    matches!(
        rust_type,
        "Vec<String>" | "std::vec::Vec<String>" | "alloc::vec::Vec<String>"
    )
}

fn pg_value_from_row_cell(
    row: &tokio_postgres::Row,
    idx: usize,
    rust_type: &str,
    is_nullable: bool,
    enum_variants: Option<&[&str]>,
) -> crate::Result<crate::model::Value> {
    if enum_variants.is_some() {
        let value: Option<PgEnumText> = pg_try_get(row, idx, rust_type)?;
        return Ok(match value {
            Some(value) => crate::model::Value::Text(value.0),
            None => {
                if is_nullable {
                    crate::model::Value::Null
                } else {
                    return Err(crate::ormer_error!(format!(
                        "Failed to parse non-nullable column at index {} (expected enum type {})",
                        idx, rust_type
                    )));
                }
            }
        });
    }

    if is_vec_i32_type(rust_type) {
        let value: Option<Vec<i32>> = pg_try_get(row, idx, "Vec<i32>")?;
        return match value {
            Some(value) => Ok(crate::model::Value::IntegerArray(value)),
            None if is_nullable => Ok(crate::model::Value::Null),
            None => Err(crate::ormer_error!(format!(
                "Failed to parse non-nullable column at index {} (expected Vec<i32> type)",
                idx
            ))),
        };
    }

    if is_vec_i64_type(rust_type) {
        let value: Option<Vec<i64>> = pg_try_get(row, idx, "Vec<i64>")?;
        return match value {
            Some(value) => Ok(crate::model::Value::BigIntArray(value)),
            None if is_nullable => Ok(crate::model::Value::Null),
            None => Err(crate::ormer_error!(format!(
                "Failed to parse non-nullable column at index {} (expected Vec<i64> type)",
                idx
            ))),
        };
    }

    if is_vec_option_i64_type(rust_type) {
        let value: Option<Vec<Option<i64>>> = pg_try_get(row, idx, "Vec<Option<i64>>")?;
        return match value {
            Some(value) => Ok(crate::model::Value::NullableBigIntArray(value)),
            None if is_nullable => Ok(crate::model::Value::Null),
            None => Err(crate::ormer_error!(format!(
                "Failed to parse non-nullable column at index {} (expected Vec<Option<i64>> type)",
                idx
            ))),
        };
    }

    if is_vec_string_type(rust_type) {
        let value: Option<Vec<String>> = pg_try_get(row, idx, "Vec<String>")?;
        return match value {
            Some(value) => Ok(crate::model::Value::TextArray(value)),
            None if is_nullable => Ok(crate::model::Value::Null),
            None => Err(crate::ormer_error!(format!(
                "Failed to parse non-nullable column at index {} (expected Vec<String> type)",
                idx
            ))),
        };
    }

    if is_nullable {
        match rust_type {
            "i8" | "i16" | "i32" | "u8" | "u16" | "u32" => {
                let value: Option<i32> = pg_try_get(row, idx, "i32")?;
                Ok(value
                    .map(|value| crate::model::Value::Integer(value as i64))
                    .unwrap_or(crate::model::Value::Null))
            }
            "i64" | "u64" => {
                let value: Option<i64> = pg_try_get(row, idx, "i64")?;
                Ok(value
                    .map(crate::model::Value::Integer)
                    .unwrap_or(crate::model::Value::Null))
            }
            "Duration" | "std::time::Duration" => {
                let value: Option<PgInterval> = pg_try_get(row, idx, "Duration")?;
                Ok(value
                    .map(|value| crate::model::Value::Duration(from_postgres_interval(value)))
                    .unwrap_or(crate::model::Value::Null))
            }
            "Uuid" | "uuid::Uuid" => {
                let value: Option<uuid::Uuid> = pg_try_get(row, idx, "uuid::Uuid")?;
                Ok(value
                    .map(crate::model::Value::Uuid)
                    .unwrap_or(crate::model::Value::Null))
            }
            "String" => {
                let value: Option<String> = pg_try_get(row, idx, "String")?;
                Ok(value
                    .map(crate::model::Value::Text)
                    .unwrap_or(crate::model::Value::Null))
            }
            "f32" | "f64" => {
                let value: Option<f64> = pg_try_get(row, idx, "f64")?;
                Ok(value
                    .map(crate::model::Value::Real)
                    .unwrap_or(crate::model::Value::Null))
            }
            "Decimal" | "rust_decimal::Decimal" => {
                let value: Option<PgNumericText> = pg_try_get(row, idx, "Decimal")?;
                Ok(value
                    .map(|value| crate::model::Value::Decimal(value.0))
                    .unwrap_or(crate::model::Value::Null))
            }
            "BigDecimal" | "bigdecimal::BigDecimal" => {
                let value: Option<PgNumericText> = pg_try_get(row, idx, "BigDecimal")?;
                Ok(value
                    .map(|value| crate::model::Value::BigDecimal(value.0))
                    .unwrap_or(crate::model::Value::Null))
            }
            "bool" => {
                let value: Option<bool> = pg_try_get(row, idx, "bool")?;
                Ok(match value {
                    Some(true) => crate::model::Value::Integer(1),
                    Some(false) => crate::model::Value::Integer(0),
                    None => crate::model::Value::Null,
                })
            }
            "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                let value: Option<Vec<u8>> = pg_try_get(row, idx, "Vec<u8>")?;
                Ok(value
                    .map(crate::model::Value::Bytes)
                    .unwrap_or(crate::model::Value::Null))
            }
            "NaiveDateTime"
            | "chrono::NaiveDateTime"
            | "DateTime"
            | "chrono::DateTime"
            | "chrono::DateTime<chrono::Utc>" => Ok(pg_datetime_value_from_row(row, idx)?
                .map(crate::model::Value::DateTime)
                .unwrap_or(crate::model::Value::Null)),
            "NaiveDate" | "chrono::NaiveDate" => Ok(pg_date_value_from_row(row, idx)?
                .map(crate::model::Value::Date)
                .unwrap_or(crate::model::Value::Null)),
            "NaiveTime" | "chrono::NaiveTime" => Ok(pg_time_value_from_row(row, idx)?
                .map(crate::model::Value::Time)
                .unwrap_or(crate::model::Value::Null)),
            _ => Err(crate::ormer_error!(
                "Unsupported nullable column type: {rust_type}"
            )),
        }
    } else {
        match rust_type {
            "i8" | "i16" | "i32" | "u8" | "u16" | "u32" => {
                let value: Option<i32> = pg_try_get(row, idx, "i32")?;
                value
                    .map(|value| crate::model::Value::Integer(value as i64))
                    .ok_or_else(|| {
                        crate::ormer_error!(format!(
                            "Failed to parse non-nullable column at index {} (expected integer type)",
                            idx
                        ))
                    })
            }
            "i64" | "u64" => {
                let value: Option<i64> = pg_try_get(row, idx, "i64")?;
                value.map(crate::model::Value::Integer).ok_or_else(|| {
                    crate::ormer_error!(format!(
                        "Failed to parse non-nullable column at index {} (expected i64 type)",
                        idx
                    ))
                })
            }
            "Duration" | "std::time::Duration" => {
                let value: Option<PgInterval> = pg_try_get(row, idx, "Duration")?;
                value
                    .map(|value| crate::model::Value::Duration(from_postgres_interval(value)))
                    .ok_or_else(|| {
                        crate::ormer_error!(format!(
                            "Failed to parse non-nullable column at index {} (expected Duration type)",
                            idx
                        ))
                    })
            }
            "Uuid" | "uuid::Uuid" => {
                let value: Option<uuid::Uuid> = pg_try_get(row, idx, "uuid::Uuid")?;
                value.map(crate::model::Value::Uuid).ok_or_else(|| {
                    crate::ormer_error!(format!(
                        "Failed to parse non-nullable column at index {} (expected uuid::Uuid type)",
                        idx
                    ))
                })
            }
            "String" => {
                let value: Option<String> = pg_try_get(row, idx, "String")?;
                value.map(crate::model::Value::Text).ok_or_else(|| {
                    crate::ormer_error!(format!(
                        "Failed to parse non-nullable column at index {} (expected String type)",
                        idx
                    ))
                })
            }
            "f32" | "f64" => {
                let value: Option<f64> = pg_try_get(row, idx, "f64")?;
                value.map(crate::model::Value::Real).ok_or_else(|| {
                    crate::ormer_error!(format!(
                        "Failed to parse non-nullable column at index {} (expected float type)",
                        idx
                    ))
                })
            }
            "Decimal" | "rust_decimal::Decimal" => {
                let value: Option<PgNumericText> = pg_try_get(row, idx, "Decimal")?;
                value
                    .map(|value| crate::model::Value::Decimal(value.0))
                    .ok_or_else(|| {
                        crate::ormer_error!(format!(
                            "Failed to parse non-nullable column at index {} (expected Decimal type)",
                            idx
                        ))
                    })
            }
            "BigDecimal" | "bigdecimal::BigDecimal" => {
                let value: Option<PgNumericText> = pg_try_get(row, idx, "BigDecimal")?;
                value
                    .map(|value| crate::model::Value::BigDecimal(value.0))
                    .ok_or_else(|| {
                        crate::ormer_error!(format!(
                            "Failed to parse non-nullable column at index {} (expected BigDecimal type)",
                            idx
                        ))
                    })
            }
            "bool" => {
                let value: Option<bool> = pg_try_get(row, idx, "bool")?;
                match value {
                    Some(true) => Ok(crate::model::Value::Integer(1)),
                    Some(false) => Ok(crate::model::Value::Integer(0)),
                    None => Err(crate::ormer_error!(format!(
                        "Failed to parse non-nullable column at index {} (expected bool type)",
                        idx
                    ))),
                }
            }
            "Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]" => {
                let value: Option<Vec<u8>> = pg_try_get(row, idx, "Vec<u8>")?;
                value.map(crate::model::Value::Bytes).ok_or_else(|| {
                    crate::ormer_error!(format!(
                        "Failed to parse non-nullable column at index {} (expected Vec<u8> type)",
                        idx
                    ))
                })
            }
            "NaiveDateTime"
            | "chrono::NaiveDateTime"
            | "DateTime"
            | "chrono::DateTime"
            | "chrono::DateTime<chrono::Utc>" => pg_datetime_value_from_row(row, idx)?
                .map(crate::model::Value::DateTime)
                .ok_or_else(|| {
                    crate::ormer_error!(format!(
                        "Failed to parse non-nullable column at index {} (expected timestamp type)",
                        idx
                    ))
                }),
            "NaiveDate" | "chrono::NaiveDate" => pg_date_value_from_row(row, idx)?
                .map(crate::model::Value::Date)
                .ok_or_else(|| {
                    crate::ormer_error!(format!(
                        "Failed to parse non-nullable column at index {} (expected date type)",
                        idx
                    ))
                }),
            "NaiveTime" | "chrono::NaiveTime" => pg_time_value_from_row(row, idx)?
                .map(crate::model::Value::Time)
                .ok_or_else(|| {
                    crate::ormer_error!(format!(
                        "Failed to parse non-nullable column at index {} (expected time type)",
                        idx
                    ))
                }),
            _ => Err(crate::ormer_error!("Unsupported column type: {rust_type}")),
        }
    }
}

fn pg_model_value_from_row<T: Model>(
    row: &tokio_postgres::Row,
    schema_idx: usize,
    row_idx: usize,
) -> crate::Result<crate::model::Value> {
    let column = &T::COLUMN_SCHEMA[schema_idx];
    let rust_type = column.data_type.unwrap_or(column.rust_type);
    pg_value_from_row_cell(
        row,
        row_idx,
        rust_type,
        column.is_nullable,
        column.enum_variants,
    )
}

fn pg_outer_join_model_value_from_row<T: Model>(
    row: &tokio_postgres::Row,
    schema_idx: usize,
    row_idx: usize,
) -> crate::Result<crate::model::Value> {
    let column = &T::COLUMN_SCHEMA[schema_idx];
    let rust_type = column.data_type.unwrap_or(column.rust_type);
    pg_value_from_row_cell(row, row_idx, rust_type, true, column.enum_variants)
}

fn pg_decode_model_from_row<T: Model>(row: &tokio_postgres::Row) -> crate::Result<T> {
    common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| {
        pg_model_value_from_row::<T>(row, i, i)
    })
}

fn pg_decode_returning_model_from_row<T: Model>(row: &tokio_postgres::Row) -> crate::Result<T> {
    common_helpers::decode_model_from_indexed_values::<T, _>(0, |i| convert_postgres_value(row, i))
}

fn pg_decode_row_values_from_row<V: crate::model::FromRowValues>(
    row: &tokio_postgres::Row,
    column_count: usize,
) -> crate::Result<V> {
    common_helpers::decode_row_values_from_indexed_values(column_count, |i| {
        convert_postgres_value(row, i)
    })
}

fn pg_collect_filter_param_rust_types<T: Model>(
    filter: &FilterExpr,
    rust_types: &mut Vec<&'static str>,
) {
    match filter {
        FilterExpr::Comparison {
            column,
            operator,
            value,
        } => {
            let rust_type = pg_model_column_rust_type::<T>(column)
                .unwrap_or_else(|| infer_filter_value_rust_type(value));
            rust_types.push(
                if operator == "@>"
                    && is_vec_string_type(rust_type)
                    && matches!(value, crate::query::filter::Value::Text(_))
                {
                    "String"
                } else {
                    rust_type
                },
            );
        }
        FilterExpr::In { column, values } | FilterExpr::NotIn { column, values } => {
            let rust_type = pg_model_column_rust_type::<T>(column);
            for value in values {
                rust_types.push(rust_type.unwrap_or_else(|| infer_filter_value_rust_type(value)));
            }
        }
        FilterExpr::InSubquery {
            subquery_params, ..
        }
        | FilterExpr::NotInSubquery {
            subquery_params, ..
        } => {
            rust_types.extend(subquery_params.iter().map(infer_model_value_rust_type));
        }
        FilterExpr::InSubqueryDynamic { subquery, .. }
        | FilterExpr::NotInSubqueryDynamic { subquery, .. }
        | FilterExpr::ExistsDynamic { subquery }
        | FilterExpr::NotExistsDynamic { subquery } => {
            rust_types.extend(
                subquery
                    .params(crate::abstract_layer::DbType::PostgreSQL)
                    .iter()
                    .map(infer_model_value_rust_type),
            );
        }
        FilterExpr::And(left, right) | FilterExpr::Or(left, right) => {
            pg_collect_filter_param_rust_types::<T>(left, rust_types);
            pg_collect_filter_param_rust_types::<T>(right, rust_types);
        }
        FilterExpr::RelationExists { filter, .. }
        | FilterExpr::ThroughRelationExists { filter, .. } => {
            if let Some(filter) = filter {
                pg_collect_filter_param_rust_types::<T>(filter, rust_types);
            }
        }
        FilterExpr::Between { column, min, max } => {
            let rust_type = pg_model_column_rust_type::<T>(column);
            rust_types.push(rust_type.unwrap_or_else(|| infer_filter_value_rust_type(min)));
            rust_types.push(rust_type.unwrap_or_else(|| infer_filter_value_rust_type(max)));
        }
        FilterExpr::Exists {
            subquery_params, ..
        }
        | FilterExpr::NotExists {
            subquery_params, ..
        } => {
            rust_types.extend(subquery_params.iter().map(infer_model_value_rust_type));
        }
        FilterExpr::ColumnComparison { .. }
        | FilterExpr::IsNull { .. }
        | FilterExpr::IsNotNull { .. }
        | FilterExpr::InvalidDynamicField { .. }
        | FilterExpr::Unsupported { .. } => {}
        FilterExpr::ExprComparison { left, right, .. } => {
            pg_collect_sql_expr_param_rust_types::<T>(left, rust_types);
            pg_collect_sql_expr_param_rust_types::<T>(right, rust_types);
        }
        FilterExpr::ExprIn { expr, values } | FilterExpr::ExprNotIn { expr, values } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            for value in values {
                pg_collect_sql_expr_param_rust_types::<T>(value, rust_types);
            }
        }
        FilterExpr::ExprBetween { expr, min, max } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            pg_collect_sql_expr_param_rust_types::<T>(min, rust_types);
            pg_collect_sql_expr_param_rust_types::<T>(max, rust_types);
        }
        FilterExpr::ExprIsNull { expr } | FilterExpr::ExprIsNotNull { expr } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
        }
        FilterExpr::ExprPredicate { expr } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
        }
        FilterExpr::TextSearch { expr, .. } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            rust_types.push("String");
        }
        FilterExpr::FullTextSearch(search) => {
            for expr in &search.exprs {
                pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            }
            rust_types.push("String");
            if search.language.is_some() {
                rust_types.push("String");
            }
        }
    }
}

fn pg_collect_sql_expr_param_rust_types<T: Model>(
    expr: &SqlExpr,
    rust_types: &mut Vec<&'static str>,
) {
    match expr {
        SqlExpr::Column(_) => {}
        SqlExpr::Raw(raw) => {
            for segment in raw.segments() {
                if let crate::query::expr::RawExprSegment::Expr(expr) = segment {
                    pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
                }
            }
        }
        SqlExpr::Value(value) => rust_types.push(infer_model_value_rust_type(value)),
        SqlExpr::Binary { left, right, .. } => {
            pg_collect_sql_expr_param_rust_types::<T>(left, rust_types);
            pg_collect_sql_expr_param_rust_types::<T>(right, rust_types);
        }
        SqlExpr::Function { args, .. } | SqlExpr::Row(args) => {
            for arg in args {
                pg_collect_sql_expr_param_rust_types::<T>(arg, rust_types);
            }
        }
        SqlExpr::WindowFunction { args, over, .. } => {
            for arg in args {
                pg_collect_sql_expr_param_rust_types::<T>(arg, rust_types);
            }
            for expr in &over.partition_by {
                pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            }
            pg_collect_order_by_param_rust_types::<T>(&over.order_by, rust_types);
        }
        SqlExpr::DateTrunc { expr, .. }
        | SqlExpr::DatePart { expr, .. }
        | SqlExpr::AtTimeZone { expr, .. } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
        }
        SqlExpr::DateAdd { expr, amount, .. } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            pg_collect_sql_expr_param_rust_types::<T>(amount, rust_types);
        }
        SqlExpr::DateDiff { left, right, .. } => {
            pg_collect_sql_expr_param_rust_types::<T>(left, rust_types);
            pg_collect_sql_expr_param_rust_types::<T>(right, rust_types);
        }
        SqlExpr::Now => {}
        SqlExpr::Cast { expr, .. }
        | SqlExpr::Collate { expr, .. }
        | SqlExpr::JsonText { expr, .. }
        | SqlExpr::JsonPathText { expr, .. }
        | SqlExpr::JsonPathValue { expr, .. }
        | SqlExpr::JsonPathExists { expr, .. }
        | SqlExpr::ArrayLen { expr } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
        }
        SqlExpr::JsonContains { left, right }
        | SqlExpr::ArrayContains { left, right }
        | SqlExpr::ArrayOverlaps { left, right } => {
            pg_collect_sql_expr_param_rust_types::<T>(left, rust_types);
            pg_collect_sql_expr_param_rust_types::<T>(right, rust_types);
        }
        SqlExpr::JsonSet { expr, value, .. } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            pg_collect_sql_expr_param_rust_types::<T>(value, rust_types);
        }
        SqlExpr::JsonRemove { expr, .. } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
        }
        SqlExpr::Aggregate {
            expr,
            filter,
            order_by,
            over,
            ..
        } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            if let Some(filter) = filter {
                pg_collect_filter_param_rust_types::<T>(filter, rust_types);
            }
            pg_collect_order_by_param_rust_types::<T>(order_by, rust_types);
            if let Some(over) = over {
                for expr in &over.partition_by {
                    pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
                }
                pg_collect_order_by_param_rust_types::<T>(&over.order_by, rust_types);
            }
        }
        SqlExpr::CaseMatch {
            expr,
            branches,
            else_expr,
        } => {
            pg_collect_sql_expr_param_rust_types::<T>(expr, rust_types);
            for (when, then) in branches {
                pg_collect_sql_expr_param_rust_types::<T>(when, rust_types);
                pg_collect_sql_expr_param_rust_types::<T>(then, rust_types);
            }
            pg_collect_sql_expr_param_rust_types::<T>(else_expr, rust_types);
        }
    }
}

fn pg_collect_order_by_param_rust_types<T: Model>(
    order_by: &[OrderBy],
    rust_types: &mut Vec<&'static str>,
) {
    for order in order_by {
        if let Some(expr) = order.cloned_expr() {
            pg_collect_sql_expr_param_rust_types::<T>(&expr, rust_types);
        }
    }
}

impl DbBackendTypeMapper for PostgreSQLTypeMapper {
    fn sql_type(
        rust_type: &str,
        is_primary: bool,
        is_auto_increment: bool,
        is_nullable: bool,
        enum_variants: Option<&[&str]>,
    ) -> String {
        // PostgreSQL 支持 ENUM 类型
        if enum_variants.is_some() {
            // 使用 rust_type 作为 ENUM 类型名（需要小蛇形命名）
            let enum_name = to_snake_case(rust_type);
            return common_helpers::sql_type_with_nullability(&enum_name, is_nullable);
        }

        // 基础类型映射
        let base_type = match rust_type {
            // 整数类型
            "i8" => "SMALLINT",
            "i16" => "SMALLINT",
            "i32" => "INTEGER",
            "i64" => "BIGINT",
            // 无符号整数（PostgreSQL 不原生支持，使用有符号类型模拟）
            "u8" => "SMALLINT",
            "u16" => "INTEGER",
            "u32" => "BIGINT",
            "u64" => "BIGINT",
            // 浮点类型
            "f32" => "REAL",
            "f64" => "DOUBLE PRECISION",
            "Decimal" | "rust_decimal::Decimal" | "BigDecimal" | "bigdecimal::BigDecimal" => {
                "NUMERIC"
            }
            // 字符串类型
            "String" => "TEXT",
            // 布尔类型
            "bool" => "BOOLEAN",
            // 时长类型
            "Duration" | "std::time::Duration" => "INTERVAL",
            // 字节数组
            "Vec<u8>" | "&[u8]" => "BYTEA",
            // PostgreSQL 原生数组
            "Vec<i32>" | "std::vec::Vec<i32>" | "alloc::vec::Vec<i32>" => "INTEGER[]",
            "Vec<i64>" | "std::vec::Vec<i64>" | "alloc::vec::Vec<i64>" => "BIGINT[]",
            "Vec<Option<i64>>" | "std::vec::Vec<Option<i64>>" | "alloc::vec::Vec<Option<i64>>" => {
                "BIGINT[]"
            }
            "Vec<String>" | "std::vec::Vec<String>" | "alloc::vec::Vec<String>" => "TEXT[]",
            // UUID 类型（如果使用 uuid crate）
            "Uuid" | "uuid::Uuid" => "UUID",
            // 日期时间类型（如果使用 chrono crate）
            "DateTime" | "chrono::DateTime" | "chrono::DateTime<chrono::Utc>" => "TIMESTAMPTZ",
            "NaiveDateTime" | "chrono::NaiveDateTime" => "TIMESTAMPTZ",
            "NaiveDate" | "chrono::NaiveDate" => "DATE",
            "NaiveTime" | "chrono::NaiveTime" => "TIME",
            // JSON 类型
            "JsonValue" | "serde_json::Value" => "JSONB",
            // 默认使用 TEXT
            _ => "TEXT",
        };

        // 首先处理主键类型（主键自动 NOT NULL）
        if is_primary {
            if is_auto_increment {
                let serial_type = match rust_type {
                    "i8" | "i16" | "i32" => "SERIAL",
                    "i64" | "u16" | "u32" | "u64" => "BIGSERIAL",
                    "u8" => "SMALLSERIAL", // PostgreSQL 最小序列类型
                    _ => "SERIAL",         // 默认使用 SERIAL
                };
                return format!("{serial_type} PRIMARY KEY");
            } else {
                return format!("{base_type} PRIMARY KEY");
            }
        }

        common_helpers::sql_type_with_nullability(base_type, is_nullable)
    }
}

/// 将驼峰命名转换为蛇形命名
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

fn postgres_fk_action(action: &str) -> Option<&'static str> {
    match action.replace('_', " ").to_ascii_uppercase().as_str() {
        "A" => Some("NO ACTION"),
        "R" => Some("RESTRICT"),
        "C" => Some("CASCADE"),
        "N" => Some("SET NULL"),
        "D" => Some("SET DEFAULT"),
        "NO ACTION" => Some("NO ACTION"),
        "RESTRICT" => Some("RESTRICT"),
        "CASCADE" => Some("CASCADE"),
        "SET NULL" => Some("SET NULL"),
        "SET DEFAULT" => Some("SET DEFAULT"),
        _ => None,
    }
}

/// QuestDB 类型映射器
///
/// QuestDB uses the PostgreSQL wire protocol, but its storage dialect has no
/// constraints, sequences, or nullable column types.
pub struct QuestDBTypeMapper;

impl DbBackendTypeMapper for QuestDBTypeMapper {
    fn sql_type(
        rust_type: &str,
        _is_primary: bool,
        is_auto_increment: bool,
        _is_nullable: bool,
        enum_variants: Option<&[&str]>,
    ) -> String {
        if is_auto_increment {
            return "LONG".to_string();
        }
        if enum_variants.is_some() {
            return "SYMBOL".to_string();
        }
        let base_type = match rust_type {
            "i8" | "i16" => "SHORT",
            "i32" | "u8" | "u16" => "INT",
            "i64" | "u32" | "u64" | "usize" | "isize" => "LONG",
            "f32" => "FLOAT",
            "f64" => "DOUBLE",
            "Decimal" | "rust_decimal::Decimal" | "BigDecimal" | "bigdecimal::BigDecimal" => {
                "DOUBLE"
            }
            "String" => "STRING",
            "bool" => "BOOLEAN",
            "Duration" | "std::time::Duration" => "LONG",
            "Vec<u8>" | "&[u8]" => "BINARY",
            "Uuid" | "uuid::Uuid" => "UUID",
            "DateTime"
            | "chrono::DateTime"
            | "chrono::DateTime<chrono::Utc>"
            | "NaiveDateTime"
            | "chrono::NaiveDateTime" => "TIMESTAMP",
            "NaiveDate" | "chrono::NaiveDate" => "DATE",
            "NaiveTime" | "chrono::NaiveTime" => "STRING",
            "JsonValue" | "serde_json::Value" => "STRING",
            _ => "STRING",
        };
        base_type.to_string()
    }
}

/// PostgreSQL 数据库连接封装
pub struct Database {
    client: std::sync::Arc<tokio_postgres::Client>,
    db_type: DbType,
}

/// 创建表执行器
pub struct CreateTableExecutor<'a, T: crate::model::WritableModel> {
    client: &'a tokio_postgres::Client,
    db_type: DbType,
    table_name: Option<String>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: crate::model::WritableModel> CreateTableExecutor<'a, T> {
    pub fn with_table_name(mut self, table_name: &str) -> Self {
        self.table_name = Some(table_name.to_string());
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let table_name = self.table_name.as_deref().unwrap_or(T::TABLE_NAME);
        let mut statements = Vec::new();

        if !self.db_type.is_questdb() {
            for column in T::COLUMN_SCHEMA.iter() {
                if let Some(variants) = column.enum_variants {
                    let enum_name = to_snake_case(column.rust_type);
                    let variants_str = variants
                        .iter()
                        .map(|v| format!("'{}'", v))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let create_enum_sql = format!(
                        "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = '{}') THEN CREATE TYPE {} AS ENUM ({}); END IF; END $$",
                        enum_name, enum_name, variants_str
                    );
                    statements.push(SingleSqlStatement::new(create_enum_sql, Vec::new()));
                }
            }
        }

        let create_sql = crate::generate_create_table_sql_with_name::<T>(
            self.db_type,
            self.table_name.as_deref(),
        )?;

        let sql_parts: Vec<&str> = create_sql.split(';').collect();
        let first_part = sql_parts[0].trim();
        if !first_part.is_empty() {
            statements.push(SingleSqlStatement::new(first_part, Vec::new()));
        }

        for sql_part in sql_parts.iter().skip(1) {
            let sql_part = sql_part.trim();
            if sql_part.is_empty() {
                continue;
            }
            statements.push(SingleSqlStatement::new(sql_part, Vec::new()));
        }

        if !self.db_type.is_questdb()
            && let Some((time_column, chunk_interval)) = T::hypertable_info()
        {
            let interval_str = chunk_interval.to_interval_string();
            let hypertable_sql = format!(
                "SELECT create_hypertable('{}', '{}', chunk_time_interval => INTERVAL '{}', if_not_exists => TRUE, migrate_data => TRUE)",
                table_name, time_column, interval_str
            );
            statements.push(SingleSqlStatement::new(hypertable_sql, Vec::new()));
        }

        Ok(SqlStatement::batch(self.db_type, statements))
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, T: crate::model::WritableModel> SqlExecutor for CreateTableExecutor<'a, T> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        CreateTableExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        for statement in sql.statements {
            traced_pg_execute_empty(self.client, &statement.sql).await?;
        }
        Ok(())
    }
}

/// 删除表执行器
pub struct DropTableExecutor<'a, T: crate::model::WritableModel> {
    client: &'a tokio_postgres::Client,
    db_type: DbType,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: crate::model::WritableModel> DropTableExecutor<'a, T> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        Ok(SqlStatement::single(
            self.db_type,
            format!(
                "DROP TABLE IF EXISTS {}{}",
                common_helpers::quote_table_name::<T>(self.db_type),
                if self.db_type.is_questdb() {
                    ""
                } else {
                    " CASCADE"
                }
            ),
            Vec::new(),
        ))
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, T: crate::model::WritableModel> SqlExecutor for DropTableExecutor<'a, T> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        DropTableExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        for statement in sql.statements {
            traced_pg_execute_empty(self.client, &statement.sql).await?;
        }
        Ok(())
    }
}

/// 插入执行器
pub struct InsertExecutor<'a, I: crate::model::Insertable> {
    db: &'a Database,
    models: I,
    conflict: Option<InsertConflict>,
    _marker: std::marker::PhantomData<I::Model>,
}

impl_insert_conflict_methods!(InsertExecutor, with_conflict);

impl<'a, I: crate::model::Insertable + Send + Sync> InsertExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        let db_type = self.db.db_type();
        if db_type.is_questdb() && self.conflict.is_some() {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "insert conflict handling",
            });
        }
        if db_type.is_questdb() && common_helpers::auto_increment_column::<I::Model>().is_some() {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: db_type,
                feature: "auto-increment insert returning",
            });
        }
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::PostgreSQL, Vec::new()));
        }

        if common_helpers::auto_increment_column::<I::Model>().is_some() {
            let (sql, all_values) = common_helpers::build_insert_statement_with_conflict::<I::Model>(
                DbType::PostgreSQL,
                &refs,
                self.conflict.as_ref(),
            )?;
            let rust_types =
                pg_insert_param_rust_types::<I::Model>(refs.len(), self.conflict.as_ref());
            let pk_col = I::Model::COLUMN_SCHEMA
                .iter()
                .find(|c| c.is_auto_increment)
                .map(|c| c.name)
                .unwrap_or("id");
            return Ok(SqlStatement::batch(
                DbType::PostgreSQL,
                vec![
                    SingleSqlStatement::new(format!("{sql} RETURNING {pk_col}"), all_values)
                        .with_param_rust_types(rust_types),
                ],
            ));
        }

        let statements = common_helpers::build_insert_statements_with_conflict::<I::Model>(
            DbType::PostgreSQL,
            &refs,
            self.conflict.as_ref(),
        )?;

        Ok(SqlStatement::batch(
            DbType::PostgreSQL,
            statements
                .into_iter()
                .map(|statement| {
                    let rust_types = pg_insert_param_rust_types::<I::Model>(
                        statement.row_count,
                        self.conflict.as_ref(),
                    );
                    SingleSqlStatement::new(statement.sql, statement.params)
                        .with_param_rust_types(rust_types)
                })
                .collect(),
        ))
    }

    /// 执行插入并返回插入的行数据（PostgreSQL RETURNING 支持）
    pub async fn returning(mut self) -> crate::Result<Vec<I::Model>> {
        if self.db.db_type().is_questdb() {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: self.db.db_type(),
                feature: "INSERT RETURNING",
            });
        }
        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;
        let mut sql = self.to_sql()?;
        if sql.statements.is_empty() {
            return Ok(Vec::new());
        }
        let mut results = Vec::new();
        for statement in &mut sql.statements {
            if let Some((prefix, _)) = statement.sql.split_once(" RETURNING ") {
                statement.sql = format!("{prefix} RETURNING *");
            } else {
                statement.sql = format!("{} RETURNING *", statement.sql);
            }

            let params = values_to_params_with_types(
                &statement.params,
                statement.param_rust_types.as_deref().unwrap_or(&[]),
            )?;
            let param_refs = pg_param_refs(&params);
            let rows = self.db.client.query(&statement.sql, &param_refs).await?;
            for row in rows {
                let model = pg_decode_returning_model_from_row::<I::Model>(&row)?;
                results.push(model);
            }
        }

        self.models.run_after_insert(hook_ctx).await?;
        Ok(results)
    }

    pub async fn execute(mut self) -> crate::Result<<I::Model as Model>::AutoIncrementKeyType> {
        let use_copy = !self.db.db_type().is_questdb() && {
            let refs = self.models.as_refs();
            pg_should_use_copy_insert::<I::Model>(&refs, self.conflict.as_ref())
        };
        if !use_copy {
            return <Self as SqlExecutor>::execute(self).await;
        }

        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;
        let refs = self.models.as_refs();

        if pg_try_copy_insert::<I::Model>(&self.db.client, &refs).await? {
            self.models.run_after_insert(hook_ctx).await?;
            return Ok(<I::Model as Model>::AutoIncrementKeyType::default());
        }

        let sql = self.to_sql()?;
        for statement in &sql.statements {
            let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
            pg_execute_with_types(
                &self.db.client,
                &statement.sql,
                &statement.params,
                rust_types,
            )
            .await?;
        }
        self.models.run_after_insert(hook_ctx).await?;
        Ok(<I::Model as Model>::AutoIncrementKeyType::default())
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for InsertExecutor<'a, I> {
    type Output = <I::Model as Model>::AutoIncrementKeyType;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if self.db.db_type().is_questdb()
            && I::Model::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment)
        {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: self.db.db_type(),
                feature: "auto-increment insert returning",
            });
        }
        if sql.statements.is_empty() {
            return Ok(<I::Model as Model>::AutoIncrementKeyType::default());
        }

        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;

        let has_auto_increment = I::Model::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        let result = if has_auto_increment {
            let statement = &sql.statements[0];
            let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
            let rows = pg_query_with_types(
                &self.db.client,
                &statement.sql,
                &statement.params,
                rust_types,
            )
            .await?;
            let row = match rows.first() {
                Some(row) => row,
                None => return Ok(Self::Output::default()),
            };
            let id: i64 = match *row.columns()[0].type_() {
                Type::INT2 => row.try_get::<_, i16>(0)? as i64,
                Type::INT4 => row.try_get::<_, i32>(0)? as i64,
                Type::INT8 => row.try_get::<_, i64>(0)?,
                _ => {
                    return Err(crate::ormer_error!(
                        "Unexpected column type for auto-increment key: {}",
                        row.columns()[0].type_()
                    ));
                }
            };
            common_helpers::convert_auto_increment_key::<Self::Output>(id)
        } else {
            for statement in &sql.statements {
                let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
                pg_execute_with_types(
                    &self.db.client,
                    &statement.sql,
                    &statement.params,
                    rust_types,
                )
                .await?;
            }
            Ok(Self::Output::default())
        }?;

        self.models.run_after_insert(hook_ctx).await?;
        Ok(result)
    }
}

pub struct InsertPartialExecutor<'a, T: Model> {
    db: &'a Database,
    assignments: Vec<InsertAssignment>,
    source_table: Option<&'static str>,
    _marker: PhantomData<T>,
}

impl<'a, T: Model> InsertPartialExecutor<'a, T> {
    fn with_assignments(mut self, assignments: Vec<InsertAssignment>) -> Self {
        self.assignments.extend(assignments);
        self
    }

    fn with_source_table(mut self, source_table: &'static str) -> Self {
        self.source_table = Some(source_table);
        self
    }

    pub fn set<F, A>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> A,
        A: IntoInsertAssignment<T>,
    {
        self.assignments
            .push(f(T::Where::default()).into_insert_assignment());
        self
    }

    pub fn default<F, C>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> C,
        C: IntoInsertDefaultColumn<T>,
    {
        self.assignments.push(InsertAssignment::default(
            f(T::Where::default()).into_insert_default_column(),
        ));
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        if self.db.db_type().is_questdb()
            && T::COLUMN_SCHEMA
                .iter()
                .any(|column| column.is_auto_increment)
        {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: self.db.db_type(),
                feature: "auto-increment insert returning",
            });
        }
        common_helpers::validate_insert_model_table::<T>(DbType::PostgreSQL, self.source_table)?;
        let statement =
            common_helpers::build_partial_insert_statement_with_auto_increment_returning::<T>(
                DbType::PostgreSQL,
                &self.assignments,
            )?;
        Ok(SqlStatement::batch(
            DbType::PostgreSQL,
            vec![
                SingleSqlStatement::new(statement.sql, statement.params)
                    .with_param_rust_types(statement.param_rust_types),
            ],
        ))
    }

    pub async fn execute(self) -> crate::Result<<T as Model>::AutoIncrementKeyType>
    where
        T: Send + Sync,
    {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, T: Model + Send + Sync> SqlExecutor for InsertPartialExecutor<'a, T> {
    type Output = <T as Model>::AutoIncrementKeyType;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertPartialExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if self.db.db_type().is_questdb()
            && T::COLUMN_SCHEMA
                .iter()
                .any(|column| column.is_auto_increment)
        {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: self.db.db_type(),
                feature: "auto-increment insert returning",
            });
        }
        if sql.statements.is_empty() {
            return Ok(<T as Model>::AutoIncrementKeyType::default());
        }

        let statement = &sql.statements[0];
        let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
        let result = if T::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment) {
            let rows = pg_query_with_types(
                &self.db.client,
                &statement.sql,
                &statement.params,
                rust_types,
            )
            .await?;
            let row = match rows.first() {
                Some(row) => row,
                None => return Ok(Self::Output::default()),
            };
            let id: i64 = match *row.columns()[0].type_() {
                Type::INT2 => row.try_get::<_, i16>(0)? as i64,
                Type::INT4 => row.try_get::<_, i32>(0)? as i64,
                Type::INT8 => row.try_get::<_, i64>(0)?,
                _ => {
                    return Err(crate::ormer_error!(
                        "Unexpected column type for auto-increment key: {}",
                        row.columns()[0].type_()
                    ));
                }
            };
            common_helpers::convert_auto_increment_key::<Self::Output>(id)
        } else {
            pg_execute_with_types(
                &self.db.client,
                &statement.sql,
                &statement.params,
                rust_types,
            )
            .await?;
            Ok(Self::Output::default())
        }?;

        Ok(result)
    }
}

/// 插入或更新执行器
pub struct InsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    db: &'a Database,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        if self.db.db_type().is_questdb() {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: self.db.db_type(),
                feature: "INSERT ON CONFLICT DO UPDATE",
            });
        }
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::PostgreSQL, Vec::new()));
        }

        let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
            DbType::PostgreSQL,
            "INSERT INTO",
            <I::Model as Model>::table_name_for_db(DbType::PostgreSQL),
            I::Model::COLUMNS,
            &refs,
            common_helpers::BatchInsertValuesMode::All,
        );
        append_postgresql_upsert_clause::<I::Model>(&mut sql, I::Model::COLUMNS);

        let rust_types: Vec<&str> = I::Model::COLUMN_SCHEMA
            .iter()
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();

        Ok(SqlStatement::batch(
            DbType::PostgreSQL,
            vec![SingleSqlStatement::new(sql, all_values).with_param_rust_types(rust_types)],
        ))
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for InsertOrUpdateExecutor<'a, I> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertOrUpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;
        for statement in sql.statements {
            let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
            pg_execute_with_types(
                &self.db.client,
                &statement.sql,
                &statement.params,
                rust_types,
            )
            .await?;
        }
        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

/// 插入或忽略执行器
pub struct InsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    db: &'a Database,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> InsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        if self.db.db_type().is_questdb() {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: self.db.db_type(),
                feature: "INSERT ON CONFLICT DO NOTHING",
            });
        }
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::PostgreSQL, Vec::new()));
        }

        let columns = I::Model::insert_columns();
        let primary_key_columns = I::Model::primary_key_columns();
        let primary_key =
            common_helpers::quote_column_list(DbType::PostgreSQL, &primary_key_columns);
        let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
            DbType::PostgreSQL,
            "INSERT INTO",
            <I::Model as Model>::table_name_for_db(DbType::PostgreSQL),
            &columns,
            &refs,
            common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
        );

        sql.push_str(&format!(" ON CONFLICT ({primary_key}) DO NOTHING"));

        let rust_types: Vec<&str> = I::Model::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();

        Ok(SqlStatement::batch(
            DbType::PostgreSQL,
            vec![SingleSqlStatement::new(sql, all_values).with_param_rust_types(rust_types)],
        ))
    }

    pub async fn execute(self) -> crate::Result<()> {
        <Self as SqlExecutor>::execute(self).await
    }
}

impl<'a, I: crate::model::Insertable + Send + Sync> SqlExecutor for InsertOrIgnoreExecutor<'a, I> {
    type Output = ();

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        InsertOrIgnoreExecutor::to_sql(self)
    }

    async fn execute_with_sql(mut self, sql: SqlStatement) -> crate::Result<Self::Output> {
        let hook_ctx = HookContext::new(HookOperation::Insert);
        self.models.run_before_insert(hook_ctx).await?;
        for statement in sql.statements {
            let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
            pg_execute_with_types(
                &self.db.client,
                &statement.sql,
                &statement.params,
                rust_types,
            )
            .await?;
        }
        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

impl Database {
    /// Connect through the PostgreSQL wire protocol.
    pub(crate) fn db_type(&self) -> DbType {
        self.db_type
    }

    pub async fn connect(db_type: super::DbType, connection_string: &str) -> crate::Result<Self> {
        let (client, connection) = tokio_postgres::connect(connection_string, NoTls)
            .trace()
            .await?;

        // 在后台运行连接
        tokio::spawn(async move {
            if let Err(err) = connection
                .trace_for("tokio_postgres::Connection::poll")
                .await
            {
                eprintln!("[ormer] {err}");
            }
        });

        if !db_type.is_questdb() {
            // 将服务端消息级别设为 WARNING，过滤掉 NOTICE/INFO/LOG/DEBUG（如 "关系已存在, 跳过"）
            traced_pg_execute_empty(&client, "SET client_min_messages TO WARNING;").await?;
        }

        Ok(Self {
            client: std::sync::Arc::new(client),
            db_type,
        })
    }

    pub(crate) async fn db_first_tables(
        &self,
        schema: Option<&str>,
    ) -> crate::Result<Vec<DbFirstTable>> {
        if self.db_type.is_questdb() {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: self.db_type,
                feature: "db-first entity generation",
            });
        }
        let schema_name = schema.filter(|value| !value.is_empty()).unwrap_or("public");
        let rows = self
            .client
            .query(
                "SELECT table_schema, table_name \
                 FROM information_schema.tables \
                 WHERE table_type = 'BASE TABLE' AND table_schema = $1 \
                   AND table_name != $2 \
                 ORDER BY table_name",
                &[&schema_name, &crate::migration::MIGRATION_TABLE_NAME],
            )
            .trace()
            .await?;
        let mut tables = Vec::with_capacity(rows.len());
        for row in rows {
            let schema_name: String = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
            let table_name: String = row.try_get(1).trace_for("tokio_postgres::Row::try_get")?;
            tables.push(self.db_first_table(&schema_name, &table_name).await?);
        }
        Ok(tables)
    }

    async fn db_first_table(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> crate::Result<DbFirstTable> {
        Ok(DbFirstTable {
            schema: Some(schema_name.to_string()),
            name: table_name.to_string(),
            columns: self.db_first_columns(schema_name, table_name).await?,
            indexes: self.db_first_indexes(schema_name, table_name).await?,
            foreign_keys: self.db_first_foreign_keys(schema_name, table_name).await?,
        })
    }

    async fn db_first_columns(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> crate::Result<Vec<DbFirstColumn>> {
        let rows = self
            .client
            .query(
                "SELECT c.column_name, \
                        CASE WHEN c.data_type = 'ARRAY' THEN c.udt_name \
                             WHEN c.data_type = 'USER-DEFINED' THEN c.udt_name \
                             ELSE c.data_type END AS type_name, \
                        c.udt_name, c.is_nullable, c.column_default, c.is_identity, \
                        EXISTS (
                            SELECT 1
                            FROM information_schema.table_constraints tc
                            JOIN information_schema.key_column_usage kcu
                              ON tc.constraint_name = kcu.constraint_name
                             AND tc.table_schema = kcu.table_schema
                             AND tc.table_name = kcu.table_name
                            WHERE tc.constraint_type = 'PRIMARY KEY'
                              AND kcu.table_schema = c.table_schema
                              AND kcu.table_name = c.table_name
                              AND kcu.column_name = c.column_name
                        ) AS is_primary
                 FROM information_schema.columns c
                 WHERE c.table_schema = $1 AND c.table_name = $2
                 ORDER BY c.ordinal_position",
                &[&schema_name, &table_name],
            )
            .trace()
            .await?;
        let mut columns = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
            let type_name: String = row.try_get(1).trace_for("tokio_postgres::Row::try_get")?;
            let udt_name: String = row.try_get(2).trace_for("tokio_postgres::Row::try_get")?;
            let nullable: String = row.try_get(3).trace_for("tokio_postgres::Row::try_get")?;
            let default: Option<String> =
                row.try_get(4).trace_for("tokio_postgres::Row::try_get")?;
            let identity: String = row.try_get(5).trace_for("tokio_postgres::Row::try_get")?;
            let primary_key: bool = row.try_get(6).trace_for("tokio_postgres::Row::try_get")?;
            let enum_variants = self
                .db_first_postgres_enum_variants(schema_name, &udt_name)
                .await?;
            let auto_increment = identity == "YES"
                || default
                    .as_deref()
                    .is_some_and(|value| value.to_ascii_lowercase().contains("nextval("));
            columns.push(DbFirstColumn {
                name,
                type_name,
                nullable: nullable == "YES" && !primary_key,
                primary_key,
                auto_increment,
                enum_variants,
                default,
            });
        }
        Ok(columns)
    }

    async fn db_first_postgres_enum_variants(
        &self,
        schema_name: &str,
        type_name: &str,
    ) -> crate::Result<Vec<String>> {
        let rows = self
            .client
            .query(
                "SELECT e.enumlabel
                 FROM pg_type t
                 JOIN pg_enum e ON e.enumtypid = t.oid
                 JOIN pg_namespace n ON n.oid = t.typnamespace
                 WHERE n.nspname = $1 AND t.typname = $2
                 ORDER BY e.enumsortorder",
                &[&schema_name, &type_name],
            )
            .trace()
            .await?;
        rows.into_iter()
            .map(|row| row.try_get(0).trace_for("tokio_postgres::Row::try_get"))
            .collect()
    }

    async fn db_first_indexes(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> crate::Result<Vec<DbFirstIndex>> {
        let rows = self
            .client
            .query(
                "SELECT i.relname, ix.indisunique, a.attname, \
                        (ix.indoption[ord.ordinality - 1] & 1) = 1 AS descending
                 FROM pg_index ix
                 JOIN pg_class i ON i.oid = ix.indexrelid
                 JOIN pg_class t ON t.oid = ix.indrelid
                 JOIN pg_namespace ns ON ns.oid = t.relnamespace
                 JOIN unnest(ix.indkey) WITH ORDINALITY AS ord(attnum, ordinality) ON TRUE
                 JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ord.attnum
                 WHERE ns.nspname = $1 AND t.relname = $2 AND NOT ix.indisprimary
                 ORDER BY i.relname, ord.ordinality",
                &[&schema_name, &table_name],
            )
            .trace()
            .await?;
        let mut indexes = std::collections::BTreeMap::<String, DbFirstIndex>::new();
        for row in rows {
            let name: String = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
            let unique: bool = row.try_get(1).trace_for("tokio_postgres::Row::try_get")?;
            let column: String = row.try_get(2).trace_for("tokio_postgres::Row::try_get")?;
            let descending: bool = row.try_get(3).trace_for("tokio_postgres::Row::try_get")?;
            indexes
                .entry(name.clone())
                .or_insert_with(|| DbFirstIndex {
                    name,
                    columns: Vec::new(),
                    unique,
                })
                .columns
                .push(DbFirstIndexColumn {
                    name: column,
                    descending,
                });
        }
        Ok(indexes.into_values().collect())
    }

    async fn db_first_foreign_keys(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> crate::Result<Vec<DbFirstForeignKey>> {
        let rows = self
            .client
            .query(
                "SELECT con.conname, local_col.attname, ref_ns.nspname, ref_table.relname, \
                        ref_col.attname, con.confdeltype::text, con.confupdtype::text
                 FROM pg_constraint con
                 JOIN pg_class local_table ON local_table.oid = con.conrelid
                 JOIN pg_namespace local_ns ON local_ns.oid = local_table.relnamespace
                 JOIN pg_class ref_table ON ref_table.oid = con.confrelid
                 JOIN pg_namespace ref_ns ON ref_ns.oid = ref_table.relnamespace
                 JOIN unnest(con.conkey) WITH ORDINALITY AS local_key(attnum, ordinality) ON TRUE
                 JOIN unnest(con.confkey) WITH ORDINALITY AS ref_key(attnum, ordinality)
                   ON ref_key.ordinality = local_key.ordinality
                 JOIN pg_attribute local_col
                   ON local_col.attrelid = local_table.oid AND local_col.attnum = local_key.attnum
                 JOIN pg_attribute ref_col
                   ON ref_col.attrelid = ref_table.oid AND ref_col.attnum = ref_key.attnum
                 WHERE con.contype = 'f' AND local_ns.nspname = $1 AND local_table.relname = $2
                 ORDER BY con.conname, local_key.ordinality",
                &[&schema_name, &table_name],
            )
            .trace()
            .await?;
        let mut foreign_keys = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
            let column: String = row.try_get(1).trace_for("tokio_postgres::Row::try_get")?;
            let ref_schema: String = row.try_get(2).trace_for("tokio_postgres::Row::try_get")?;
            let ref_table: String = row.try_get(3).trace_for("tokio_postgres::Row::try_get")?;
            let ref_column: String = row.try_get(4).trace_for("tokio_postgres::Row::try_get")?;
            let on_delete: String = row.try_get(5).trace_for("tokio_postgres::Row::try_get")?;
            let on_update: String = row.try_get(6).trace_for("tokio_postgres::Row::try_get")?;
            foreign_keys.push(DbFirstForeignKey {
                name: Some(name),
                column,
                ref_schema: Some(ref_schema),
                ref_table,
                ref_column,
                on_delete: postgres_fk_action(&on_delete).map(str::to_string),
                on_update: postgres_fk_action(&on_update).map(str::to_string),
            });
        }
        Ok(foreign_keys)
    }

    /// 从 bb8 PooledConnection 创建 Database
    ///
    /// bb8-postgres 的 PooledConnection 通过 Deref 提供 &Client 访问。
    /// 由于 tokio_postgres::Client 不实现 Clone，我们使用 std::ops::Deref
    /// 获取引用后，通过 unsafe ptr::read 复制 Client（Client 内部使用 Arc，
    /// 复制是安全的，只是增加 Arc 引用计数），然后 forget PooledConnection
    /// 防止其 drop 时关闭连接。
    pub fn from_pooled_connection(
        db_type: DbType,
        pooled: bb8::PooledConnection<'_, bb8_postgres::PostgresConnectionManager<NoTls>>,
    ) -> Self {
        use std::ops::Deref;
        let client_ref: &tokio_postgres::Client = pooled.deref();
        let client = unsafe { std::ptr::read(client_ref as *const _) };
        std::mem::forget(pooled);
        Self {
            client: std::sync::Arc::new(client),
            db_type,
        }
    }

    /// 创建表 - 返回执行器
    pub fn create_table<T: WritableModel>(&self) -> CreateTableExecutor<'_, T> {
        CreateTableExecutor {
            client: &self.client,
            db_type: self.db_type,
            table_name: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(&self, models: I) -> InsertExecutor<'_, I> {
        InsertExecutor {
            db: self,
            models,
            conflict: None,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn insert_partial<T: WritableModel>(&self) -> InsertPartialExecutor<'_, T> {
        InsertPartialExecutor {
            db: self,
            assignments: Vec::new(),
            source_table: None,
            _marker: PhantomData,
        }
    }

    pub fn insert_model<T>(
        &self,
        model: impl crate::model::InsertModel<T>,
    ) -> InsertPartialExecutor<'_, T>
    where
        T: WritableModel,
    {
        self.insert_partial::<T>()
            .with_source_table(model.insert_table_name())
            .with_assignments(model.insert_assignments())
    }

    /// 插入或更新记录 - 返回执行器
    pub fn insert_or_update<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrUpdateExecutor<'_, I> {
        InsertOrUpdateExecutor {
            db: self,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入或忽略记录 - 返回执行器（存在重复主键时忽略）
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &self,
        models: I,
    ) -> InsertOrIgnoreExecutor<'_, I> {
        InsertOrIgnoreExecutor {
            db: self,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 验证表结构是否与模型定义匹配
    pub async fn validate_table<T: WritableModel>(&self) -> crate::Result<()> {
        if self.db_type.is_questdb() {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: self.db_type,
                feature: "schema introspection",
            });
        }
        // 检查表是否存在
        let table_exists = self.check_table_exists::<T>().trace().await?;

        if !table_exists {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Table does not exist",
                T::TABLE_NAME
            ));
        }

        // 表已存在，验证表结构
        self.validate_table_schema::<T>().await?;
        self.validate_table_hypertable::<T>().await
    }

    /// 检查表是否存在
    async fn check_table_exists<T: Model>(&self) -> crate::Result<bool> {
        let (schema_name, table_name) = split_schema_table_name(T::TABLE_NAME, "public");
        let sql = "SELECT COUNT(*) FROM information_schema.tables WHERE table_type='BASE TABLE' AND table_schema=$1 AND table_name=$2";

        let row = self
            .client
            .query_one(sql, &[&schema_name, &table_name])
            .trace()
            .await?;

        let count: i64 = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;

        Ok(count > 0)
    }

    async fn check_table_is_hypertable<T: Model>(&self) -> crate::Result<bool> {
        let sql = "SELECT to_regclass('timescaledb_information.hypertables') IS NOT NULL";
        let row = self.client.query_one(sql, &[]).trace().await?;
        let has_hypertables_view: bool =
            row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;

        if !has_hypertables_view {
            return Ok(false);
        }

        let (schema_name, table_name) = split_schema_table_name(T::TABLE_NAME, "public");
        let sql = r#"
            SELECT COUNT(*)
            FROM timescaledb_information.hypertables
            WHERE hypertable_schema = $1 AND hypertable_name = $2
        "#;
        let row = self
            .client
            .query_one(sql, &[&schema_name, &table_name])
            .trace()
            .await?;
        let count: i64 = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;

        Ok(count > 0)
    }

    async fn check_hypertable_dimensions<T: Model>(
        &self,
        time_column: &str,
        chunk_interval: std::time::Duration,
    ) -> crate::Result<(i64, i64)> {
        let sql = "SELECT to_regclass('timescaledb_information.dimensions') IS NOT NULL";
        let row = self.client.query_one(sql, &[]).trace().await?;
        let has_dimensions_view: bool = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;

        if !has_dimensions_view {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: TimescaleDB dimensions metadata is unavailable",
                T::TABLE_NAME
            ));
        }

        let (schema_name, table_name) = split_schema_table_name(T::TABLE_NAME, "public");
        let expected_interval = to_postgres_interval(chunk_interval);
        let sql = r#"
            SELECT
                COUNT(*)::BIGINT,
                COUNT(*) FILTER (
                    WHERE column_name = $3
                      AND time_interval = $4::interval
                )::BIGINT
            FROM timescaledb_information.dimensions
            WHERE hypertable_schema = $1 AND hypertable_name = $2
        "#;
        let row = self
            .client
            .query_one(
                sql,
                &[&schema_name, &table_name, &time_column, &expected_interval],
            )
            .trace()
            .await?;
        let dimension_count: i64 = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
        let matching_dimension_count: i64 =
            row.try_get(1).trace_for("tokio_postgres::Row::try_get")?;

        Ok((dimension_count, matching_dimension_count))
    }

    async fn validate_table_hypertable<T: Model>(&self) -> crate::Result<()> {
        let expected_hypertable = T::hypertable_info();
        let actual_hypertable = self.check_table_is_hypertable::<T>().trace().await?;

        if expected_hypertable.is_some() != actual_hypertable {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Hypertable mismatch: expected {}, but actual is {}",
                T::TABLE_NAME,
                if expected_hypertable.is_some() {
                    "hypertable"
                } else {
                    "regular table"
                },
                if actual_hypertable {
                    "hypertable"
                } else {
                    "regular table"
                }
            ));
        }

        if let Some((time_column, chunk_interval)) = expected_hypertable {
            let (dimension_count, matching_dimension_count) = self
                .check_hypertable_dimensions::<T>(time_column, chunk_interval)
                .await?;

            if dimension_count != 1 || matching_dimension_count != 1 {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Hypertable dimension mismatch: expected one time dimension on column '{}' with chunk interval '{}', but actual dimensions={} matching_dimensions={}",
                    T::TABLE_NAME,
                    time_column,
                    chunk_interval.to_interval_string(),
                    dimension_count,
                    matching_dimension_count
                ));
            }
        }

        Ok(())
    }

    pub(crate) async fn validate_hypertable_for_migration<T: Model>(&self) -> crate::Result<()> {
        self.validate_table_hypertable::<T>().await
    }

    /// 验证表结构是否与模型定义匹配（内部使用）
    async fn validate_table_schema<T: Model>(&self) -> crate::Result<()> {
        // 查询表的列信息
        let (schema_name, table_name) = split_schema_table_name(T::TABLE_NAME, "public");
        let sql = r#"
            SELECT column_name, data_type, udt_name, is_nullable,
                   EXISTS (
                       SELECT 1
                       FROM information_schema.table_constraints tc
                       JOIN information_schema.key_column_usage kcu
                         ON tc.constraint_name = kcu.constraint_name
                        AND tc.table_schema = kcu.table_schema
                        AND tc.table_name = kcu.table_name
                       WHERE tc.constraint_type = 'PRIMARY KEY'
                         AND kcu.table_schema = c.table_schema
                         AND kcu.table_name = c.table_name
                         AND kcu.column_name = c.column_name
                   ) AS is_primary,
                   COALESCE(column_default LIKE 'nextval(%', FALSE) AS is_auto_increment,
                   CASE a.attcompression
                       WHEN 'p' THEN 'pglz'
                       WHEN 'l' THEN 'lz4'
                       ELSE NULL
                   END AS compression
            FROM information_schema.columns c
            JOIN pg_namespace ns
              ON ns.nspname = c.table_schema
            JOIN pg_class rel
              ON rel.relnamespace = ns.oid AND rel.relname = c.table_name
            JOIN pg_attribute a
              ON a.attrelid = rel.oid
             AND a.attname = c.column_name
             AND a.attnum > 0
             AND NOT a.attisdropped
            WHERE c.table_schema = $1 AND c.table_name = $2
            ORDER BY c.ordinal_position
        "#;

        let rows = self
            .client
            .query(sql, &[&schema_name, &table_name])
            .trace()
            .await?;

        // 收集实际的表结构
        let mut actual_columns: Vec<(String, String, bool, bool, bool, Option<String>)> =
            Vec::new();
        for row in rows {
            let name: String = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
            let col_type: String = row.try_get(1).trace_for("tokio_postgres::Row::try_get")?;
            let udt_name: String = row.try_get(2).trace_for("tokio_postgres::Row::try_get")?;
            let is_nullable: String = row.try_get(3).trace_for("tokio_postgres::Row::try_get")?;
            let is_primary: bool = row.try_get(4).trace_for("tokio_postgres::Row::try_get")?;
            let is_auto_increment: bool =
                row.try_get(5).trace_for("tokio_postgres::Row::try_get")?;
            let compression: Option<String> =
                row.try_get(6).trace_for("tokio_postgres::Row::try_get")?;

            let actual_type = if col_type == "USER-DEFINED" || col_type == "ARRAY" {
                udt_name
            } else {
                col_type
            };
            actual_columns.push((
                name,
                actual_type,
                is_nullable == "YES",
                is_primary,
                is_auto_increment,
                compression,
            ));
        }

        // 比较列数量
        if actual_columns.len() != T::COLUMNS.len() {
            return Err(crate::ormer_error!(
                "Schema mismatch: table {}, reason: Column count mismatch: expected {}, but actual is {}",
                T::TABLE_NAME,
                T::COLUMNS.len(),
                actual_columns.len()
            ));
        }

        // 比较每一列的定义
        for (i, expected_col) in T::COLUMN_SCHEMA.iter().enumerate() {
            if i >= actual_columns.len() {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Missing column: {}",
                    T::TABLE_NAME,
                    expected_col.name
                ));
            }

            let (
                actual_name,
                actual_type,
                actual_nullable,
                actual_primary,
                actual_auto_increment,
                actual_compression,
            ) = &actual_columns[i];

            // 检查列名
            if actual_name != expected_col.name {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Column name mismatch at position {}: expected '{}', but actual is '{}'",
                    T::TABLE_NAME,
                    i,
                    expected_col.name,
                    actual_name
                ));
            }

            if expected_col.is_primary != *actual_primary {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Primary key mismatch for '{}': expected {}primary key, but actual is {}primary key",
                    T::TABLE_NAME,
                    expected_col.name,
                    if expected_col.is_primary { "" } else { "not " },
                    if *actual_primary { "" } else { "not " }
                ));
            }

            if expected_col.is_auto_increment != *actual_auto_increment {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Auto-increment mismatch for '{}': expected {}, but actual is {}",
                    T::TABLE_NAME,
                    expected_col.name,
                    expected_col.is_auto_increment,
                    actual_auto_increment
                ));
            }

            let effective_rust_type = expected_col.data_type.unwrap_or(expected_col.rust_type);

            // 检查列类型（只比较基础类型，不包含约束）
            let expected_type = crate::abstract_layer::DbType::PostgreSQL.sql_type(
                effective_rust_type,
                expected_col.is_primary,
                expected_col.is_auto_increment,
                expected_col.is_nullable,
                expected_col.enum_variants,
            );

            // 对于类型比较，我们需要提取基础类型（不包含 SERIAL, PRIMARY KEY, NOT NULL 等约束）
            let type_to_compare = if expected_col.is_primary && expected_col.is_auto_increment {
                // SERIAL类型在PostgreSQL中实际存储为integer/bigint
                match effective_rust_type {
                    "i8" | "i16" | "u8" => "SMALLINT".to_string(), // SMALLSERIAL -> SMALLINT
                    "i32" | "u16" | "u32" => "INTEGER".to_string(), // SERIAL -> INTEGER
                    "i64" | "u64" => "BIGINT".to_string(),         // BIGSERIAL -> BIGINT
                    _ => "INTEGER".to_string(),
                }
            } else if expected_col.is_primary {
                // 主键的基础类型
                match effective_rust_type {
                    "i8" | "i16" | "u8" => "SMALLINT".to_string(),
                    "i32" | "u16" | "u32" => "INTEGER".to_string(),
                    "i64" | "u64" => "BIGINT".to_string(),
                    // 非整数主键（如 NaiveDateTime）使用 sql_type 获取基础类型
                    _ => {
                        let full_type = crate::abstract_layer::DbType::PostgreSQL.sql_type(
                            effective_rust_type,
                            false,
                            expected_col.is_auto_increment,
                            expected_col.is_nullable,
                            expected_col.enum_variants,
                        );
                        full_type.replace(" NOT NULL", "")
                    }
                }
            } else {
                // 非主键列，提取基础类型（去掉 NOT NULL）
                let full_type = crate::abstract_layer::DbType::PostgreSQL.sql_type(
                    effective_rust_type,
                    false,
                    expected_col.is_auto_increment,
                    expected_col.is_nullable,
                    expected_col.enum_variants,
                );
                // 去掉 " NOT NULL" 后缀
                full_type.replace(" NOT NULL", "")
            };

            if !Self::types_compatible(actual_type, &type_to_compare) {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Column type mismatch for '{}': expected '{expected_type}', but actual is '{actual_type}'",
                    T::TABLE_NAME,
                    expected_col.name
                ));
            }

            // 检查 NOT NULL 约束（主键列除外，因为主键自动 NOT NULL）
            if !expected_col.is_primary {
                let expected_nullable = expected_col.is_nullable;
                if *actual_nullable != expected_nullable {
                    return Err(crate::ormer_error!(
                        "Schema mismatch: table {}, reason: Column nullability mismatch for '{}': expected {}NULL, but actual is {}NULL",
                        T::TABLE_NAME,
                        expected_col.name,
                        if expected_nullable { "" } else { "NOT " },
                        if *actual_nullable { "" } else { "NOT " }
                    ));
                }
            }

            let expected_compression = crate::model::column_compression_algorithm(expected_col)
                .map(|value| value.as_str());
            if actual_compression.as_deref() != expected_compression {
                return Err(crate::ormer_error!(
                    "Schema mismatch: table {}, reason: Compression mismatch for '{}': expected {}, but actual is {}",
                    T::TABLE_NAME,
                    expected_col.name,
                    expected_compression.unwrap_or("default"),
                    actual_compression.as_deref().unwrap_or("default")
                ));
            }
        }

        let actual_table = self.db_first_table(&schema_name, &table_name).await?;
        crate::db_first::validate_model_constraints::<T>(
            crate::abstract_layer::DbType::PostgreSQL,
            &actual_table,
        )?;
        Ok(())
    }

    /// 检查 SQL 类型是否兼容
    fn types_compatible(actual: &str, expected: &str) -> bool {
        // 标准化类型名称 - 只提取基础类型，去除约束
        fn normalize(s: &str) -> String {
            let upper = s.to_uppercase();
            match upper.as_str() {
                "_INT4" | "INT4[]" | "INTEGER[]" => return "INTEGER[]".to_string(),
                "_INT8" | "INT8[]" | "BIGINT[]" => return "BIGINT[]".to_string(),
                "_TEXT"
                | "TEXT[]"
                | "_VARCHAR"
                | "VARCHAR[]"
                | "_BPCHAR"
                | "CHAR[]"
                | "CHARACTER VARYING[]" => return "TEXT[]".to_string(),
                _ => {}
            }
            if upper.starts_with("TIMESTAMP WITH TIME ZONE") || upper == "TIMESTAMPTZ" {
                return "TIMESTAMPTZ".to_string();
            }
            if upper.starts_with("TIMESTAMP WITHOUT TIME ZONE") || upper == "TIMESTAMP" {
                return "TIMESTAMP".to_string();
            }
            // 提取第一个单词作为基础类型
            let base_type = upper.split_whitespace().next().unwrap_or(&upper);

            match base_type {
                // 整数类型
                "SMALLINT" | "INT2" => "SMALLINT".to_string(),
                "INTEGER" | "INT" | "INT4" | "SERIAL" => "INTEGER".to_string(),
                "BIGINT" | "INT8" | "BIGSERIAL" => "BIGINT".to_string(),
                // 字符串类型
                "CHARACTER" => {
                    // CHARACTER VARYING 需要特殊处理
                    if upper.starts_with("CHARACTER VARYING") || upper.starts_with("CHARACTER(") {
                        "VARCHAR".to_string()
                    } else {
                        "CHAR".to_string()
                    }
                }
                "VARCHAR" | "TEXT" | "CHAR" | "BPCHAR" => "VARCHAR".to_string(),
                // 布尔类型
                "BOOLEAN" | "BOOL" => "BOOLEAN".to_string(),
                // 浮点类型
                "REAL" | "FLOAT4" => "REAL".to_string(),
                "DOUBLE" => "DOUBLE PRECISION".to_string(), // DOUBLE PRECISION
                "FLOAT8" | "FLOAT" => "DOUBLE PRECISION".to_string(),
                // 字节类型
                "BYTEA" | "BLOB" => "BYTEA".to_string(),
                // 其他
                _ => base_type.to_string(),
            }
        }

        let actual = normalize(actual);
        let expected = normalize(expected);
        actual == expected
            || matches!(
                (actual.as_str(), expected.as_str()),
                ("TIMESTAMP", "TIMESTAMPTZ") | ("TIMESTAMPTZ", "TIMESTAMP")
            )
    }

    /// 批量插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update_batch<T: Model>(&self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        // 构建批量插入或更新的 SQL: INSERT INTO table (cols) VALUES (...), (...) ON CONFLICT (primary_keys) DO UPDATE SET ...
        let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<T>(
            DbType::PostgreSQL,
            "INSERT INTO",
            T::table_name_for_db(DbType::PostgreSQL),
            T::COLUMNS,
            models,
            common_helpers::BatchInsertValuesMode::All,
        );

        // 添加 ON CONFLICT DO UPDATE 子句
        append_postgresql_upsert_clause::<T>(&mut sql, T::COLUMNS);

        // 获取列的rust_type信息
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();
        pg_execute_with_types(&self.client, &sql, &all_values, &rust_types).await?;
        Ok(())
    }

    /// 批量插入或忽略记录（遇到重复键时忽略）
    pub async fn insert_or_ignore_batch<T: Model>(&self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        let columns = T::insert_columns();
        let primary_key_columns = T::primary_key_columns();
        let primary_key =
            common_helpers::quote_column_list(DbType::PostgreSQL, &primary_key_columns);

        // 构建批量插入或忽略的 SQL: INSERT INTO table (cols) VALUES (...), (...) ON CONFLICT (primary_key) DO NOTHING
        let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<T>(
            DbType::PostgreSQL,
            "INSERT INTO",
            T::table_name_for_db(DbType::PostgreSQL),
            &columns,
            models,
            common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
        );

        // 添加 ON CONFLICT DO NOTHING 子句
        sql.push_str(&format!(" ON CONFLICT ({primary_key}) DO NOTHING"));

        // 获取列的rust_type信息（排除自增主键，优先使用data_type覆盖）
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();
        pg_execute_with_types(&self.client, &sql, &all_values, &rust_types).await?;
        Ok(())
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: WritableModel>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            versioned: false,
            questdb: self.db_type.is_questdb().then_some(self.db_type),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: WritableModel>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            model_updates: Vec::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Related 查询执行器（关联查询）
    pub fn related<T: Model + 'static, R: Model>(&self) -> RelatedSelectExecutor<'_, T, R> {
        RelatedSelectExecutor {
            select: Select::<T>::new().from::<T, R>(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 开始事务
    pub async fn begin(&self) -> crate::Result<Transaction<'_>> {
        if self.db_type.is_questdb() {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend: self.db_type,
                feature: "transactions",
            });
        }
        traced_pg_execute_empty(&self.client, "BEGIN").await?;
        Ok(Transaction {
            client: std::sync::Arc::clone(&self.client),
            state: common_helpers::TransactionState::Active,
            _marker: std::marker::PhantomData,
        })
    }

    /// 删除表 - 返回执行器
    pub fn drop_table<T: WritableModel>(&self) -> DropTableExecutor<'_, T> {
        DropTableExecutor {
            client: &self.client,
            db_type: self.db_type,
            _marker: std::marker::PhantomData,
        }
    }

    /// 执行原生非查询 SQL 并返回影响的行数
    pub async fn execute_sql(&self, sql: impl IntoRawSql) -> crate::Result<u64> {
        let sql = sql.into_raw_sql();
        let (sql, params) = sql.render(DbType::PostgreSQL)?;
        self.exec_raw(&sql, params).await
    }

    pub(crate) async fn select_raw<V, C>(&self, sql: &str, params: Vec<Value>) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
        C: FromIterator<V>,
    {
        let rows = pg_query_untyped(&self.client, sql, &params).await?;
        let mut results = Vec::new();
        for row in rows {
            results.push(pg_decode_row_values_from_row(&row, row.columns().len())?);
        }
        Ok(results.into_iter().collect())
    }

    pub(crate) async fn select_raw_with_types<V, C>(
        &self,
        sql: &str,
        params: Vec<Value>,
        rust_types: Vec<&'static str>,
    ) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
        C: FromIterator<V>,
    {
        let rows = pg_query_for_query(&self.client, sql, &params, &rust_types).await?;
        let mut results = Vec::new();
        for row in rows {
            results.push(pg_decode_row_values_from_row(&row, row.columns().len())?);
        }
        Ok(results.into_iter().collect())
    }

    pub(crate) async fn exec_raw(&self, sql: &str, params: Vec<Value>) -> crate::Result<u64> {
        pg_execute_untyped(&self.client, sql, &params).await
    }

    pub(crate) async fn migration_history(&self) -> crate::Result<Vec<(u64, String, u64)>> {
        let sql = if self.db_type.is_questdb() {
            // QuestDB's PG-wire query cache does not invalidate after DDL in the
            // same session, so each history read needs a distinct statement key.
            let cache_key = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default();
            format!(
                "SELECT version, name, checksum FROM __ormer_migrations \
                 WHERE rolled_back = FALSE ORDER BY version /* {cache_key} */"
            )
        } else {
            "SELECT version, name, checksum FROM __ormer_migrations ORDER BY version".to_string()
        };
        let rows = self.client.query(&sql, &[]).trace().await?;
        rows.into_iter()
            .map(|row| {
                let version: i64 = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                let name: String = row.try_get(1).trace_for("tokio_postgres::Row::try_get")?;
                let checksum = row
                    .try_get::<_, String>(2)
                    .trace_for("tokio_postgres::Row::try_get")?
                    .parse::<u64>()
                    .map_err(|_| {
                        crate::ormer_error!("Migration checksum is not a valid unsigned integer")
                    })?;
                Ok((
                    u64::try_from(version)
                        .map_err(|_| crate::ormer_error!("Migration version cannot be negative"))?,
                    name,
                    checksum,
                ))
            })
            .collect()
    }

    pub(crate) async fn schema_columns(
        &self,
        table_name: &str,
    ) -> crate::Result<Option<Vec<SchemaColumn>>> {
        let (schema_name, table_name) = split_schema_table_name(table_name, "public");
        let exists = self
            .client
            .query_one(
                "SELECT EXISTS (
                    SELECT 1 FROM information_schema.tables
                    WHERE table_schema = $1 AND table_name = $2
                )",
                &[&schema_name, &table_name],
            )
            .trace()
            .await?
            .try_get::<_, bool>(0)
            .trace_for("tokio_postgres::Row::try_get")?;
        if !exists {
            return Ok(None);
        }
        let rows = self
            .client
            .query(
                "SELECT c.column_name, c.data_type, c.is_nullable, \
                        EXISTS (
                            SELECT 1
                            FROM information_schema.table_constraints tc
                            JOIN information_schema.key_column_usage kcu
                              ON tc.constraint_name = kcu.constraint_name
                             AND tc.table_schema = kcu.table_schema
                             AND tc.table_name = kcu.table_name
                            WHERE tc.constraint_type = 'PRIMARY KEY'
                              AND kcu.table_schema = c.table_schema
                              AND kcu.table_name = c.table_name
                              AND kcu.column_name = c.column_name
                        ) AS is_primary,
                        CASE a.attcompression
                            WHEN 'p' THEN 'pglz'
                            WHEN 'l' THEN 'lz4'
                            ELSE NULL
                        END AS compression
                 FROM information_schema.columns c
                 JOIN pg_namespace ns
                   ON ns.nspname = c.table_schema
                 JOIN pg_class rel
                   ON rel.relnamespace = ns.oid AND rel.relname = c.table_name
                 JOIN pg_attribute a
                   ON a.attrelid = rel.oid
                  AND a.attname = c.column_name
                  AND a.attnum > 0
                  AND NOT a.attisdropped
                 WHERE c.table_schema = $1 AND c.table_name = $2
                 ORDER BY c.ordinal_position",
                &[&schema_name, &table_name],
            )
            .trace()
            .await?;
        let mut columns = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
            let type_name: String = row.try_get(1).trace_for("tokio_postgres::Row::try_get")?;
            let nullable: String = row.try_get(2).trace_for("tokio_postgres::Row::try_get")?;
            let primary_key: bool = row.try_get(3).trace_for("tokio_postgres::Row::try_get")?;
            let compression: Option<String> =
                row.try_get(4).trace_for("tokio_postgres::Row::try_get")?;
            columns.push(schema_column_with_compression(
                name,
                type_name,
                nullable == "YES",
                primary_key,
                compression,
            ));
        }
        Ok(Some(columns))
    }

    /// 检查连接是否有效
    pub async fn is_valid(&self) -> bool {
        traced_pg_execute_empty(&self.client, "SELECT 1")
            .await
            .is_ok()
    }
}

/// PostgreSQL 事务对象
pub struct Transaction<'a> {
    client: std::sync::Arc<tokio_postgres::Client>,
    state: common_helpers::TransactionState,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> Drop for Transaction<'a> {
    fn drop(&mut self) {
        if !self.state.is_active() {
            return;
        }

        self.state = common_helpers::TransactionState::RolledBack;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let client = std::sync::Arc::clone(&self.client);
            handle.spawn(async move {
                let _ = traced_pg_execute_empty(&client, "ROLLBACK").await;
            });
        }
    }
}

/// 事务中的插入执行器
pub struct TransactionInsertExecutor<'a, I: crate::model::Insertable> {
    client: &'a tokio_postgres::Client,
    models: I,
    conflict: Option<InsertConflict>,
    _marker: std::marker::PhantomData<I::Model>,
}

impl_insert_conflict_methods!(TransactionInsertExecutor);

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::PostgreSQL, Vec::new()));
        }

        if common_helpers::auto_increment_column::<I::Model>().is_some() {
            let (sql, all_values) = common_helpers::build_insert_statement_with_conflict::<I::Model>(
                DbType::PostgreSQL,
                &refs,
                self.conflict.as_ref(),
            )?;
            let rust_types =
                pg_insert_param_rust_types::<I::Model>(refs.len(), self.conflict.as_ref());
            let pk_col = I::Model::COLUMN_SCHEMA
                .iter()
                .find(|c| c.is_auto_increment)
                .map(|c| c.name)
                .unwrap_or("id");
            return Ok(SqlStatement::batch(
                DbType::PostgreSQL,
                vec![
                    SingleSqlStatement::new(format!("{sql} RETURNING {pk_col}"), all_values)
                        .with_param_rust_types(rust_types),
                ],
            ));
        }

        let statements = common_helpers::build_insert_statements_with_conflict::<I::Model>(
            DbType::PostgreSQL,
            &refs,
            self.conflict.as_ref(),
        )?;

        Ok(SqlStatement::batch(
            DbType::PostgreSQL,
            statements
                .into_iter()
                .map(|statement| {
                    let rust_types = pg_insert_param_rust_types::<I::Model>(
                        statement.row_count,
                        self.conflict.as_ref(),
                    );
                    SingleSqlStatement::new(statement.sql, statement.params)
                        .with_param_rust_types(rust_types)
                })
                .collect(),
        ))
    }

    pub async fn execute(mut self) -> crate::Result<<I::Model as Model>::AutoIncrementKeyType> {
        let use_copy = {
            let refs = self.models.as_refs();
            pg_should_use_copy_insert::<I::Model>(&refs, self.conflict.as_ref())
        };
        if use_copy {
            let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
            self.models.run_before_insert(hook_ctx).await?;
            let refs = self.models.as_refs();
            if pg_try_copy_insert::<I::Model>(self.client, &refs).await? {
                self.models.run_after_insert(hook_ctx).await?;
                return Ok(<<I::Model as Model>::AutoIncrementKeyType>::default());
            }

            let sql = self.to_sql()?;
            for statement in &sql.statements {
                let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
                pg_execute_with_types(self.client, &statement.sql, &statement.params, rust_types)
                    .await?;
            }
            self.models.run_after_insert(hook_ctx).await?;
            return Ok(<<I::Model as Model>::AutoIncrementKeyType>::default());
        }

        let sql = self.to_sql()?;
        if sql.statements.is_empty() {
            return Ok(<<I::Model as Model>::AutoIncrementKeyType>::default());
        }
        let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
        self.models.run_before_insert(hook_ctx).await?;
        let has_auto_increment = I::Model::COLUMN_SCHEMA.iter().any(|c| c.is_auto_increment);
        let result = if has_auto_increment {
            let statement = &sql.statements[0];
            let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
            let rows =
                pg_query_with_types(self.client, &statement.sql, &statement.params, rust_types)
                    .await?;
            let row = match rows.first() {
                Some(row) => row,
                None => return Ok(<<I::Model as Model>::AutoIncrementKeyType>::default()),
            };
            // 根据列类型读取自增主键值（SERIAL = INT4, BIGSERIAL = INT8）
            let id: i64 = match *row.columns()[0].type_() {
                Type::INT2 => row.try_get::<_, i16>(0)? as i64,
                Type::INT4 => row.try_get::<_, i32>(0)? as i64,
                Type::INT8 => row.try_get::<_, i64>(0)?,
                _ => {
                    return Err(crate::ormer_error!(
                        "Unexpected column type for auto-increment key: {}",
                        row.columns()[0].type_()
                    ));
                }
            };
            common_helpers::convert_auto_increment_key::<<I::Model as Model>::AutoIncrementKeyType>(
                id,
            )
        } else {
            for statement in &sql.statements {
                let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
                pg_execute_with_types(self.client, &statement.sql, &statement.params, rust_types)
                    .await?;
            }
            Ok(<<I::Model as Model>::AutoIncrementKeyType>::default())
        }?;

        self.models.run_after_insert(hook_ctx).await?;
        Ok(result)
    }
}

/// 事务中的插入或更新执行器
pub struct TransactionInsertOrUpdateExecutor<'a, I: crate::model::Insertable> {
    client: &'a tokio_postgres::Client,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertOrUpdateExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::PostgreSQL, Vec::new()));
        }
        let columns = I::Model::insert_columns();
        let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
            DbType::PostgreSQL,
            "INSERT INTO",
            <I::Model as Model>::table_name_for_db(DbType::PostgreSQL),
            &columns,
            &refs,
            common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
        );
        append_postgresql_upsert_clause::<I::Model>(&mut sql, &columns);
        let rust_types: Vec<&str> = I::Model::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();
        Ok(SqlStatement::batch(
            DbType::PostgreSQL,
            vec![SingleSqlStatement::new(sql, all_values).with_param_rust_types(rust_types)],
        ))
    }

    pub async fn execute(mut self) -> crate::Result<()> {
        let sql = self.to_sql()?;
        if sql.statements.is_empty() {
            return Ok(());
        }
        let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
        self.models.run_before_insert(hook_ctx).await?;
        let statement = &sql.statements[0];
        let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
        pg_execute_with_types(self.client, &statement.sql, &statement.params, rust_types).await?;
        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

/// 事务中的插入或忽略执行器
pub struct TransactionInsertOrIgnoreExecutor<'a, I: crate::model::Insertable> {
    client: &'a tokio_postgres::Client,
    models: I,
    _marker: std::marker::PhantomData<I::Model>,
}

impl<'a, I: crate::model::Insertable + Send + Sync> TransactionInsertOrIgnoreExecutor<'a, I> {
    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let refs = self.models.as_refs();
        if refs.is_empty() {
            return Ok(SqlStatement::batch(DbType::PostgreSQL, Vec::new()));
        }
        let columns = I::Model::insert_columns();
        let primary_key_columns = I::Model::primary_key_columns();
        let primary_key =
            common_helpers::quote_column_list(DbType::PostgreSQL, &primary_key_columns);
        let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<I::Model>(
            DbType::PostgreSQL,
            "INSERT INTO",
            <I::Model as Model>::table_name_for_db(DbType::PostgreSQL),
            &columns,
            &refs,
            common_helpers::BatchInsertValuesMode::WithoutAutoIncrement,
        );
        sql.push_str(&format!(" ON CONFLICT ({}) DO NOTHING", primary_key));

        let rust_types: Vec<&str> = I::Model::COLUMN_SCHEMA
            .iter()
            .filter(|col| !col.is_auto_increment)
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();

        Ok(SqlStatement::batch(
            DbType::PostgreSQL,
            vec![SingleSqlStatement::new(sql, all_values).with_param_rust_types(rust_types)],
        ))
    }

    pub async fn execute(mut self) -> crate::Result<()> {
        let sql = self.to_sql()?;
        if sql.statements.is_empty() {
            return Ok(());
        }
        let hook_ctx = HookContext::new(HookOperation::Insert).transaction();
        self.models.run_before_insert(hook_ctx).await?;
        let statement = &sql.statements[0];
        let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
        pg_execute_with_types(self.client, &statement.sql, &statement.params, rust_types).await?;
        self.models.run_after_insert(hook_ctx).await?;
        Ok(())
    }
}

impl<'a> Transaction<'a> {
    pub(crate) async fn exec_raw(&mut self, sql: &str, params: Vec<Value>) -> crate::Result<u64> {
        pg_execute_untyped(&self.client, sql, &params).await
    }

    pub(crate) async fn select_raw<V, C>(&self, sql: &str, params: Vec<Value>) -> crate::Result<C>
    where
        V: crate::model::FromRowValues,
        C: FromIterator<V>,
    {
        let rows = pg_query_untyped(&self.client, sql, &params).await?;
        let mut results = Vec::new();
        for row in rows {
            results.push(pg_decode_row_values_from_row(&row, row.columns().len())?);
        }
        Ok(results.into_iter().collect())
    }

    /// 提交事务
    pub async fn commit(mut self) -> crate::Result<()> {
        if self.state.is_closed() {
            return Err(crate::ormer_error!(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        traced_pg_execute_empty(&self.client, "COMMIT").await?;
        self.state = common_helpers::TransactionState::Committed;
        Ok(())
    }

    /// 回滚事务
    pub async fn rollback(mut self) -> crate::Result<()> {
        if self.state.is_closed() {
            return Err(crate::ormer_error!(
                "Transaction already committed or rolled back".to_string(),
            ));
        }
        traced_pg_execute_empty(&self.client, "ROLLBACK").await?;
        self.state = common_helpers::TransactionState::RolledBack;
        Ok(())
    }

    /// 关闭并回滚事务
    pub async fn close(self) -> crate::Result<()> {
        self.rollback().await
    }

    /// 创建 Select 查询执行器
    pub fn select<T: Model>(&self) -> SelectExecutor<'_, T> {
        SelectExecutor {
            select: Select::<T>::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建分组聚合查询执行器
    pub fn select_column<T: Model, V>(&self) -> GroupedSelectExecutor<'_, T, V> {
        GroupedSelectExecutor {
            select: GroupedSelect::<T, V>::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Delete 执行器
    pub fn delete<T: WritableModel>(&self) -> DeleteExecutor<'_, T> {
        DeleteExecutor {
            filters: Vec::new(),
            versioned: false,
            questdb: None,
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 创建 Update 执行器
    pub fn update<T: WritableModel>(&self) -> UpdateExecutor<'_, T> {
        UpdateExecutor {
            sets: Vec::new(),
            filters: Vec::new(),
            model_updates: Vec::new(),
            client: &self.client,
            _marker: PhantomData,
        }
    }

    /// 插入记录 - 返回执行器
    pub fn insert<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertExecutor<'_, I> {
        TransactionInsertExecutor {
            client: &self.client,
            models,
            conflict: None,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入或更新记录 - 返回执行器
    pub fn insert_or_update<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrUpdateExecutor<'_, I> {
        TransactionInsertOrUpdateExecutor {
            client: &self.client,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 插入或忽略记录 - 返回执行器（存在重复主键时忽略）
    pub fn insert_or_ignore<I: crate::model::Insertable>(
        &mut self,
        models: I,
    ) -> TransactionInsertOrIgnoreExecutor<'_, I> {
        TransactionInsertOrIgnoreExecutor {
            client: &self.client,
            models,
            _marker: std::marker::PhantomData,
        }
    }

    /// 批量插入或更新记录（遇到重复键时更新）
    pub async fn insert_or_update_batch<T: Model>(&self, models: &[&T]) -> crate::Result<()> {
        if models.is_empty() {
            return Ok(());
        }

        // 构建批量插入或更新的 SQL: INSERT INTO table (cols) VALUES (...), (...) ON CONFLICT (primary_keys) DO UPDATE SET ...
        let (mut sql, all_values) = common_helpers::build_batch_insert_statement::<T>(
            DbType::PostgreSQL,
            "INSERT INTO",
            T::table_name_for_db(DbType::PostgreSQL),
            T::COLUMNS,
            models,
            common_helpers::BatchInsertValuesMode::All,
        );

        // 添加 ON CONFLICT DO UPDATE 子句
        append_postgresql_upsert_clause::<T>(&mut sql, T::COLUMNS);

        // 获取列的rust_type信息
        let rust_types: Vec<&str> = T::COLUMN_SCHEMA
            .iter()
            .map(|col| col.data_type.unwrap_or(col.rust_type))
            .collect();
        pg_execute_with_types(&self.client, &sql, &all_values, &rust_types).await?;

        Ok(())
    }
}

/// LEFT JOIN 查询执行器
pub struct LeftJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: LeftJoinedSelect<T, J>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, J)>,
}

/// INNER JOIN 查询执行器
pub struct InnerJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: InnerJoinedSelect<T, J>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, J)>,
}

/// RIGHT JOIN 查询执行器
pub struct RightJoinedSelectExecutor<'a, T: Model, J: Model> {
    select: RightJoinedSelect<T, J>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, J)>,
}

/// Select 查询执行器
pub struct SelectExecutor<'a, T: Model> {
    select: Select<T>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<T>,
}

/// 映射查询结果执行器
pub struct MappedSelectExecutor<'a, T: Model, V> {
    select: crate::query::builder::MappedSelect<T, V>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, V)>,
}

/// 分组聚合查询执行器
pub struct GroupedSelectExecutor<'a, T: Model, V> {
    select: GroupedSelect<T, V>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, V)>,
}

impl<'a, T: Model, V> MappedSelectExecutor<'a, T, V> {
    /// 生成子查询SQL和参数
    pub fn to_subquery_sql(&self) -> crate::Result<(String, Vec<crate::model::Value>)> {
        self.select.try_to_sql_with_params(DbType::PostgreSQL)
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> MappedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
    {
        MappedCollectFuture {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn as_model<R: Model>(self) -> crate::query::builder::DerivedSelect<R>
    where
        T: Send + Sync + 'static,
        V: Send + Sync + 'static,
    {
        self.select.as_model::<R>()
    }

    /// 克隆executor（保持相同的client引用）
    pub fn clone_with_client(&self) -> Self {
        Self {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }
}

/// 映射查询收集Future
pub struct MappedCollectFuture<'a, T: Model, V, C> {
    select: crate::query::builder::MappedSelect<T, V>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, V, C)>,
}

impl<
    'a,
    T: Model + 'static + Send,
    V: crate::model::FromRowValues + 'static + Send,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for MappedCollectFuture<'a, T, V, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let param_rust_types = self.select.param_rust_types();
            let (sql, params) = self.select.try_to_sql_with_params(DbType::PostgreSQL)?;
            let rows = pg_query_for_query(self.client, &sql, &params, &param_rust_types).await?;

            let mut results = Vec::new();
            for row in rows {
                let v = pg_decode_row_values_from_row(&row, row.columns().len())?;
                results.push(v);
            }

            Ok(results.into_iter().collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CompositePkModel;

    impl Model for CompositePkModel {
        const TABLE_NAME: &'static str = "composite_pk_models";
        const COLUMNS: &'static [&'static str] = &["tenant_id", "user_id", "role"];
        const COLUMN_SCHEMA: &'static [crate::model::ColumnSchema] = &[];

        type AutoIncrementKeyType = ();
        type QueryBuilder = ();
        type Where = ();
        type Update = ();

        fn query() -> Self::QueryBuilder {}

        fn select() -> Self::QueryBuilder {}

        fn from_row(_row: &Row) -> crate::Result<Self> {
            unreachable!()
        }

        fn from_row_values(_values: &[Value]) -> crate::Result<Self> {
            unreachable!()
        }

        fn field_values(&self) -> Vec<Value> {
            Vec::new()
        }

        fn primary_key_columns() -> &'static [&'static str] {
            &["tenant_id", "user_id"]
        }

        fn primary_key_values(&self) -> Vec<Value> {
            Vec::new()
        }
    }

    #[test]
    fn upsert_clause_uses_all_primary_key_columns() {
        let mut sql =
            "INSERT INTO composite_pk_models (tenant_id, user_id, role) VALUES ($1, $2, $3)"
                .to_string();

        append_postgresql_upsert_clause::<CompositePkModel>(&mut sql, CompositePkModel::COLUMNS);

        assert!(sql.contains("ON CONFLICT (tenant_id, user_id) DO UPDATE SET"));
        assert!(sql.contains("role = EXCLUDED.role"));
        assert!(!sql.contains("tenant_id = EXCLUDED.tenant_id"));
        assert!(!sql.contains("user_id = EXCLUDED.user_id"));
    }

    #[test]
    fn nullable_duration_null_is_bound_as_interval() {
        let params = values_to_params_with_types(&[Value::Null], &["std::time::Duration"]).unwrap();
        let mut out = BytesMut::new();

        let is_null = params[0]
            .to_sql_checked(&PgType::INTERVAL, &mut out)
            .unwrap();

        assert!(matches!(is_null, IsNull::Yes));
    }

    #[test]
    fn datetime_parameters_support_timestamp_and_timestamptz() {
        let value = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();

        for ty in [PgType::TIMESTAMP, PgType::TIMESTAMPTZ] {
            let mut out = BytesMut::new();
            let is_null = PgDateTimeParam(value)
                .to_sql_checked(&ty, &mut out)
                .unwrap();

            assert!(matches!(is_null, IsNull::No));
            assert!(!out.is_empty());
        }
    }

    #[test]
    fn timestamp_and_timestamptz_schema_types_are_compatible() {
        assert!(Database::types_compatible(
            "timestamp without time zone",
            "TIMESTAMPTZ"
        ));
        assert!(Database::types_compatible(
            "timestamp with time zone",
            "TIMESTAMP"
        ));
        assert!(!Database::types_compatible("DATE", "TIMESTAMPTZ"));
    }
}

impl_backend_executor_methods!(SelectExecutor, client, &'a tokio_postgres::Client, Select);

impl<'a, T: Model> SelectExecutor<'a, T> {
    pub(crate) fn select_model<R: Model>(&self) -> SelectExecutor<'a, R> {
        SelectExecutor {
            select: Select::new().with_context_filters(self.select.context_filters()),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 克隆executor（保持相同的client引用）
    pub fn clone_with_client(&self) -> Self {
        Self {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加 LEFT JOIN 查询
    pub fn left_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<'a, T, J> {
        LeftJoinedSelectExecutor {
            select: self.select.left_join::<J>(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加 INNER JOIN 查询
    pub fn inner_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<'a, T, J> {
        InnerJoinedSelectExecutor {
            select: self.select.inner_join::<J>(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加 RIGHT JOIN 查询
    pub fn right_join<J: Model>(
        self,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<'a, T, J> {
        RightJoinedSelectExecutor {
            select: self.select.right_join::<J>(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn left_join_derived<J: Model>(
        self,
        derived: crate::query::builder::DerivedSelect<J>,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> LeftJoinedSelectExecutor<'a, T, J> {
        LeftJoinedSelectExecutor {
            select: self.select.left_join_derived::<J>(derived, f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn inner_join_derived<J: Model>(
        self,
        derived: crate::query::builder::DerivedSelect<J>,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> InnerJoinedSelectExecutor<'a, T, J> {
        InnerJoinedSelectExecutor {
            select: self.select.inner_join_derived::<J>(derived, f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn right_join_derived<J: Model>(
        self,
        derived: crate::query::builder::DerivedSelect<J>,
        f: impl FnOnce(T::Where, J::Where) -> WhereExpr,
    ) -> RightJoinedSelectExecutor<'a, T, J> {
        RightJoinedSelectExecutor {
            select: self.select.right_join_derived::<J>(derived, f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 映射查询结果到自定义类型
    pub fn map_to<F, M>(self, f: F) -> MappedSelectExecutor<'a, T, M::Output>
    where
        F: FnOnce(T::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        let mapped_select = self.select.map_to(f);
        MappedSelectExecutor {
            select: mapped_select,
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 忽略指定字段，查询时用默认常量替代真实列值
    pub fn ignore<F, M>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> M,
        M: crate::query::builder::MapToResult,
    {
        Self {
            select: self.select.ignore(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 选择列（支持聚合函数）- 转换为分组查询
    pub fn select_column<F, V>(self, f: F) -> GroupedSelectExecutor<'a, T, V>
    where
        F: FnOnce(T::Where) -> V,
        V: crate::query::builder::SelectColumnResult,
    {
        let grouped_select = self.select.select_column(f);
        GroupedSelectExecutor {
            select: grouped_select,
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<T> + 'static>(&self) -> CollectFuture<'a, T, C> {
        CollectFuture {
            executor: self.clone_with_client(),
            _marker: PhantomData,
        }
    }

    /// 执行查询并返回第一条记录
    pub fn first(self) -> FirstFuture<'a, T> {
        FirstFuture { executor: self }
    }

    /// COUNT 聚合函数
    pub fn count<F, C>(self, f: F) -> AggregateFuture<'a, T, usize>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
    {
        let aggregate_select = self.select.count(f);
        AggregateFuture {
            aggregate_select,
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// SUM 聚合函数
    pub fn sum<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.sum(f);
        AggregateFuture {
            aggregate_select,
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// AVG 聚合函数
    pub fn avg<F, C>(self, f: F) -> AggregateFuture<'a, T, Option<f64>>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.avg(f);
        AggregateFuture {
            aggregate_select,
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// MAX 聚合函数
    pub fn max<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.max(f);
        AggregateFuture {
            aggregate_select,
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// MIN 聚合函数
    pub fn min<F, C>(self, f: F) -> AggregateFuture<'a, T, C::Output>
    where
        F: FnOnce(<T as Model>::Where) -> crate::query::builder::TypedColumn<C, T>,
        C: crate::query::builder::AggregateResultType + 'static,
    {
        let aggregate_select = self.select.min(f);
        AggregateFuture {
            aggregate_select,
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持2个泛型参数，第一个必须与T相同）
    /// select::<User>().from::<User, Role>()
    pub fn from<T2, R: Model>(self) -> RelatedSelectExecutor<'a, T, R>
    where
        T2: Model + 'static,
    {
        RelatedSelectExecutor {
            select: self.select.from::<T2, R>(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持3个表）
    /// select::<User>().from3::<User, Role, Permission>()
    pub fn from3<T2, R1: Model, R2: Model>(self) -> MultiTableSelectExecutor<'a, T, R1, R2>
    where
        T2: Model + 'static,
    {
        MultiTableSelectExecutor {
            select: self.select.from3::<T2, R1, R2>(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加关联表查询（支持4个表）
    /// select::<User>().from4::<User, Role, Permission, Department>()
    pub fn from4<T2, R1: Model, R2: Model, R3: Model>(
        self,
    ) -> FourTableSelectExecutor<'a, T, R1, R2, R3>
    where
        T2: Model + 'static,
    {
        FourTableSelectExecutor {
            select: self.select.from4::<T2, R1, R2, R3>(),
            client: self.client,
            _marker: PhantomData,
        }
    }
}

/// Collect future - 允许 .collect::<Vec<_>>().await 语法
pub struct CollectFuture<'a, T: Model, C: FromIterator<T>> {
    executor: SelectExecutor<'a, T>,
    _marker: PhantomData<C>,
}

/// First future for单条记录查询
pub struct FirstFuture<'a, T: Model> {
    executor: SelectExecutor<'a, T>,
}

/// Aggregate future for聚合函数执行
pub struct AggregateFuture<'a, T: Model, R> {
    aggregate_select: crate::query::builder::AggregateSelect<T, R>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, R)>,
}

impl<'a, T: Model + 'static + Send, R: crate::model::FromValue + 'static + Send>
    std::future::IntoFuture for AggregateFuture<'a, T, R>
{
    type Output = crate::Result<R>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (mut sql, params) = self.aggregate_select.to_sql_with_params(DbType::PostgreSQL);

            // 对于 AVG 聚合,PostgreSQL 返回 NUMERIC 类型,需要 CAST 为 FLOAT8
            // 这样可以避免 tokio-postgres 不支持 NUMERIC 类型的问题
            if sql.contains("SELECT AVG(") {
                // 在 AVG 函数的闭括号后添加 ::FLOAT8
                // 找到 "AVG(column_name)" 并替换为 "AVG(column_name)::FLOAT8"
                if let Some(avg_start) = sql.find("AVG(") {
                    if let Some(paren_end) = sql[avg_start..].find(')') {
                        let insert_pos = avg_start + paren_end + 1;
                        sql.insert_str(insert_pos, "::FLOAT8");
                    }
                }
            }

            let pg_params = values_to_params_auto_integer(&params);

            let params_ref = pg_param_refs(&pg_params);

            let row = self.client.query_one(&sql, &params_ref).trace().await?;

            // 获取第一列的值
            use tokio_postgres::types::Type;
            let column_type = row.columns()[0].type_();

            // 根据类型获取值
            let ormer_value = match *column_type {
                Type::INT2 => {
                    let val: Option<i16> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(|v| crate::model::Value::Integer(v as i64))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::INT4 => {
                    let val: Option<i32> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(|v| crate::model::Value::Integer(v as i64))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::INT8 => {
                    let val: Option<i64> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(crate::model::Value::Integer)
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::INTERVAL => {
                    let val: Option<PgInterval> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(|v| crate::model::Value::Duration(from_postgres_interval(v)))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::UUID => {
                    let val: Option<uuid::Uuid> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(crate::model::Value::Uuid)
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::FLOAT4 => {
                    let val: Option<f32> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(|v| crate::model::Value::Real(v as f64))
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::FLOAT8 => {
                    let val: Option<f64> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(crate::model::Value::Real)
                        .unwrap_or(crate::model::Value::Null)
                }
                Type::NUMERIC => {
                    let val_result: Result<Option<PgNumericText>, _> = row.try_get(0);
                    match val_result {
                        Ok(Some(v)) => crate::model::Value::BigDecimal(v.0),
                        Ok(None) => crate::model::Value::Null,
                        Err(_) => crate::model::Value::Null,
                    }
                }
                Type::TEXT | Type::VARCHAR => {
                    let val: Option<String> =
                        row.try_get(0).trace_for("tokio_postgres::Row::try_get")?;
                    val.map(crate::model::Value::Text)
                        .unwrap_or(crate::model::Value::Null)
                }
                _ => crate::model::Value::Null,
            };

            // 使用 FromValue 转换为目标类型
            R::from_value(&ormer_value)
        })
    }
}

impl<'a, T: Model + 'static + Send, C: FromIterator<T> + 'static> std::future::IntoFuture
    for CollectFuture<'a, T, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model + 'static + Send + std::marker::Sync> std::future::IntoFuture
    for FirstFuture<'a, T>
{
    type Output = crate::Result<Option<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let results: Vec<T> = self.executor.collect_inner().await?;
            Ok(results.into_iter().next())
        })
    }
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    async fn collect_inner<C: FromIterator<T>>(self) -> crate::Result<C> {
        let (sql, params) = self.select.try_to_sql_with_params(DbType::PostgreSQL)?;
        let param_rust_types = self.select.param_rust_types();

        let rows = pg_query_for_query(self.client, &sql, &params, &param_rust_types).await?;

        let mut results = Vec::new();

        for row in rows {
            let model = pg_decode_model_from_row::<T>(&row)?;
            results.push(model);
        }

        Ok(results.into_iter().collect())
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let (sql, params) = self.select.try_to_sql_with_params(DbType::PostgreSQL)?;
        let rust_types = self.select.param_rust_types();
        Ok(SqlStatement::batch(
            DbType::PostgreSQL,
            vec![SingleSqlStatement::new(sql, params).with_param_rust_types(rust_types)],
        ))
    }
}

/// Delete 执行器
pub struct DeleteExecutor<'a, T: Model> {
    filters: Vec<FilterExpr>,
    versioned: bool,
    questdb: Option<DbType>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<T>,
}

impl<'a, T: Model> DeleteExecutor<'a, T> {
    /// 添加 WHERE 条件
    pub fn filter<F, W>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> W,
        W: Into<WhereExpr>,
    {
        let where_obj = T::Where::default();
        let expr = crate::query::filter::FilterExpr::from(f(where_obj).into());
        self.filters.push(expr);
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        if let Some(backend) = self.questdb {
            return Err(crate::OrmerError::UnsupportedFeature {
                backend,
                feature: "row delete",
            });
        }
        let (sql, params) = self.build_sql_with_params();
        let rust_types = self.filter_param_rust_types();
        Ok(SqlStatement::batch(
            DbType::PostgreSQL,
            vec![
                SingleSqlStatement::new(sql, params)
                    .with_param_rust_types(rust_types)
                    .with_optimistic_lock(self.versioned, None),
            ],
        ))
    }

    pub fn model(mut self, model: &T) -> Self {
        self.filters
            .extend(common_helpers::model_delete_filters(model));
        self.versioned = T::version_info().is_some();
        self
    }

    /// 执行删除操作并返回影响的行数
    pub async fn execute(self) -> crate::Result<u64> {
        <Self as SqlExecutor>::execute(self).await
    }

    /// 执行删除并返回被删除的行数据（PostgreSQL RETURNING 支持）
    pub async fn returning(self) -> crate::Result<Vec<T>> {
        let mut sql = self.to_sql()?;
        if sql.statements.is_empty() {
            return Ok(Vec::new());
        }

        let statement = &mut sql.statements[0];
        statement.sql = format!("{} RETURNING *", statement.sql);

        let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
        let rows =
            pg_query_with_types(self.client, &statement.sql, &statement.params, rust_types).await?;

        let mut results = Vec::new();
        for row in rows {
            let model = pg_decode_returning_model_from_row::<T>(&row)?;
            results.push(model);
        }

        Ok(results)
    }

    fn build_sql_with_params(&self) -> (String, Vec<Value>) {
        common_helpers::build_delete_sql::<T>(DbType::PostgreSQL, &self.filters)
            .unwrap_or_else(|err| panic!("Failed to build delete SQL: {}", err))
    }

    fn filter_param_rust_types(&self) -> Vec<&'static str> {
        let mut rust_types = Vec::new();
        for filter in &self.filters {
            pg_collect_filter_param_rust_types::<T>(filter, &mut rust_types);
        }
        rust_types
    }
}

impl<'a, T: Model> SqlExecutor for DeleteExecutor<'a, T> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        DeleteExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        if sql.statements.is_empty() {
            return Ok(0);
        }

        let statement = &sql.statements[0];
        let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
        let result =
            pg_execute_with_types(self.client, &statement.sql, &statement.params, rust_types)
                .await?;
        if statement.versioned && result == 0 {
            return Err(common_helpers::optimistic_lock_conflict::<T>());
        }
        Ok(result)
    }
}

impl<'a, T: Model + 'static + Send> std::future::IntoFuture for DeleteExecutor<'a, T> {
    type Output = crate::Result<u64>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

/// Update 执行器
pub struct UpdateExecutor<'a, T: Model> {
    sets: Vec<UpdateAssignment>,
    filters: Vec<FilterExpr>,
    model_updates: ModelUpdateBatch,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<T>,
}

impl<'a, T: Model> UpdateExecutor<'a, T> {
    /// 添加 WHERE 条件
    pub fn filter<F, W>(mut self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> W,
        W: Into<WhereExpr>,
    {
        let where_obj = T::Where::default();
        let expr = crate::query::filter::FilterExpr::from(f(where_obj).into());
        self.filters.push(expr);
        self
    }

    /// 设置要更新的字段
    pub fn set<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut T::Update),
    {
        let mut update = T::Update::default();
        f(&mut update);
        self.sets
            .extend(<T::Update as crate::query::update::UpdateFields>::assignments(&update));
        self
    }

    /// 从模型实例设置所有非主键字段，并自动添加主键作为 WHERE 条件
    ///
    /// ```ignore
    /// let user = User { id: 1, name: "Bob".into(), age: 25, email: Some("bob@test.com".into()) };
    /// db.update::<User>().set_model(&user).execute().await?;
    /// ```
    pub fn set_model(mut self, model: &T) -> Self {
        if let Some(plan) = common_helpers::model_update_plan(model, None) {
            self.model_updates.push(plan);
        }
        self
    }

    pub fn set_model_fields(mut self, model: &T, fields: &[String]) -> Self {
        if let Some(plan) = common_helpers::model_update_plan(model, Some(fields)) {
            self.model_updates.push(plan);
        }
        self
    }

    pub fn to_sql(&self) -> crate::Result<SqlStatement> {
        let statements = self.build_all_sql()?;
        let mut sql_statements = Vec::with_capacity(statements.len());
        for (statement, rust_types) in statements {
            sql_statements.push(
                SingleSqlStatement::new(statement.sql, statement.params)
                    .with_param_rust_types(rust_types)
                    .with_optimistic_lock(statement.versioned, statement.version_update),
            );
        }
        Ok(SqlStatement::batch(DbType::PostgreSQL, sql_statements))
    }

    /// 执行更新操作
    pub async fn execute(self) -> crate::Result<u64> {
        <Self as SqlExecutor>::execute(self).await
    }

    /// 执行更新并返回被更新的行数据（PostgreSQL RETURNING 支持）
    pub async fn returning(self) -> crate::Result<Vec<T>> {
        let sql = self.to_sql()?;
        let mut results = Vec::new();
        for statement in &sql.statements {
            let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
            let returning_sql = format!("{} RETURNING *", statement.sql);
            let rows =
                pg_query_with_types(self.client, &returning_sql, &statement.params, rust_types)
                    .await?;
            for row in rows {
                let model = pg_decode_returning_model_from_row::<T>(&row)?;
                results.push(model);
            }
        }
        Ok(results)
    }

    fn build_all_sql(&self) -> crate::Result<UpdateSqlBatch> {
        let mut statements = Vec::new();

        // Base UPDATE from sets/filters
        if !self.sets.is_empty() || (self.model_updates.is_empty() && !self.filters.is_empty()) {
            let mut rust_types = Vec::new();
            for assignment in &self.sets {
                pg_collect_update_assignment_param_rust_types::<T>(assignment, &mut rust_types);
            }
            for filter in &self.filters {
                pg_collect_filter_param_rust_types::<T>(filter, &mut rust_types);
            }
            let (sql, params) = common_helpers::build_update_sql::<T>(
                DbType::PostgreSQL,
                &self.sets,
                &self.filters,
            )?;
            statements.push((
                common_helpers::ModelSqlStatement {
                    sql,
                    params,
                    versioned: false,
                    version_update: None,
                    param_columns: None,
                },
                rust_types,
            ));
        }

        if let Some(batch_statements) = common_helpers::build_bulk_model_update_statements::<T>(
            DbType::PostgreSQL,
            &self.model_updates,
        )? {
            for statement in batch_statements {
                let rust_types =
                    pg_model_statement_param_rust_types::<T>(&statement).unwrap_or_default();
                statements.push((statement, rust_types));
            }
        } else {
            for plan in &self.model_updates {
                let mut rust_types = Vec::new();
                for (col_name, value) in &plan.sets {
                    rust_types.push(
                        pg_model_column_rust_type::<T>(col_name)
                            .unwrap_or_else(|| infer_model_value_rust_type(value)),
                    );
                }
                for filter in &plan.filters {
                    pg_collect_filter_param_rust_types::<T>(filter, &mut rust_types);
                }
                statements.push((
                    common_helpers::build_model_update_sql::<T>(DbType::PostgreSQL, plan)?,
                    rust_types,
                ));
            }
        }

        Ok(statements)
    }
}

impl<'a, T: Model> SqlExecutor for UpdateExecutor<'a, T> {
    type Output = u64;

    fn to_sql(&self) -> crate::Result<SqlStatement> {
        UpdateExecutor::to_sql(self)
    }

    async fn execute_with_sql(self, sql: SqlStatement) -> crate::Result<Self::Output> {
        let mut total: u64 = 0;
        for statement in &sql.statements {
            let rust_types = statement.param_rust_types.as_deref().unwrap_or(&[]);
            let result =
                pg_execute_with_types(self.client, &statement.sql, &statement.params, rust_types)
                    .await?;
            if statement.versioned && result == 0 {
                return Err(common_helpers::optimistic_lock_conflict::<T>());
            }
            if result > 0 {
                if let Some(update) = &statement.version_update {
                    update.apply();
                }
            }
            total += result;
        }
        Ok(total)
    }
}

impl<'a, T: Model + 'static + Send> std::future::IntoFuture for UpdateExecutor<'a, T> {
    type Output = crate::Result<u64>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.execute().await })
    }
}

fn pg_value_to_param(value: &Value, rust_type: Option<&str>) -> PostgreSQLParam {
    match value {
        Value::Integer(value) => match rust_type {
            Some(rust_type) if matches!(rust_type, "i64" | "u64") => Box::new(*value),
            Some(rust_type)
                if matches!(
                    rust_type,
                    "i8" | "i16" | "i32" | "u8" | "u16" | "u32" | "usize" | "isize"
                ) =>
            {
                Box::new(*value as i32)
            }
            Some(_) => Box::new(value.to_string()),
            None => Box::new(*value as i32),
        },
        Value::Text(value) => match rust_type {
            Some(rust_type) if is_vec_string_type(rust_type) => {
                Box::new(crate::model::parse_string_vec_text(value))
            }
            Some(_) => Box::new(PgTextParam::from(value.clone())),
            None => Box::new(value.clone()),
        },
        Value::TextArray(value) => match rust_type {
            Some(rust_type) if is_vec_string_type(rust_type) => {
                Box::new(crate::model::normalize_string_vec(value.clone()))
            }
            _ => Box::new(value.clone()),
        },
        Value::Real(value) => Box::new(*value),
        Value::Decimal(value) | Value::BigDecimal(value) => {
            Box::new(PgNumericTextParam(value.clone()))
        }
        Value::Boolean(value) => Box::new(*value),
        Value::Bytes(value) => Box::new(value.clone()),
        Value::IntegerArray(value) => Box::new(value.clone()),
        Value::BigIntArray(value) => Box::new(value.clone()),
        Value::NullableBigIntArray(value) => Box::new(value.clone()),
        Value::Duration(value) => Box::new(to_postgres_interval(*value)),
        Value::DateTime(value) => match rust_type {
            Some(_) => Box::new(PgDateTimeParam(*value)),
            None => Box::new(*value),
        },
        Value::Date(value) => Box::new(*value),
        Value::Time(value) => Box::new(*value),
        Value::Json(value) => Box::new(value.to_string()),
        Value::Uuid(value) => Box::new(*value),
        Value::BigInt(value) => Box::new(*value as i64),
        Value::Null => match rust_type {
            None => Box::new(None::<i32>),
            Some(rust_type) if is_vec_i32_type(rust_type) => Box::new(None::<Vec<i32>>),
            Some(rust_type) if is_vec_i64_type(rust_type) => Box::new(None::<Vec<i64>>),
            Some(rust_type) if is_vec_option_i64_type(rust_type) => {
                Box::new(None::<Vec<Option<i64>>>)
            }
            Some(rust_type) if is_vec_string_type(rust_type) => Box::new(None::<Vec<String>>),
            Some("i64" | "u64") => Box::new(None::<i64>),
            Some("i32" | "i16" | "i8" | "u16" | "u32" | "u8") => Box::new(None::<i32>),
            Some("String" | "&str") => Box::new(PgMaybeTextParam(None)),
            Some("f32" | "f64") => Box::new(None::<f64>),
            Some("Decimal" | "rust_decimal::Decimal" | "BigDecimal" | "bigdecimal::BigDecimal") => {
                Box::new(PgMaybeNumericTextParam(None))
            }
            Some("Duration" | "std::time::Duration") => Box::new(None::<PgInterval>),
            Some("Uuid" | "uuid::Uuid") => Box::new(None::<uuid::Uuid>),
            Some("bool") => Box::new(None::<bool>),
            Some("Vec<u8>" | "std::vec::Vec<u8>" | "alloc::vec::Vec<u8>" | "&[u8]") => {
                Box::new(None::<Vec<u8>>)
            }
            Some(
                "DateTime"
                | "chrono::DateTime"
                | "chrono::DateTime<chrono::Utc>"
                | "NaiveDateTime"
                | "chrono::NaiveDateTime",
            ) => Box::new(PgMaybeDateTimeParam(None)),
            Some("NaiveDate" | "chrono::NaiveDate") => Box::new(None::<chrono::NaiveDate>),
            Some("NaiveTime" | "chrono::NaiveTime") => Box::new(None::<chrono::NaiveTime>),
            Some(_) => Box::new(PgMaybeTextParam(None)),
        },
    }
}

/// 将 ormer Value 转换为 tokio-postgres 参数，并按列类型选择整数和 NULL 类型。
fn values_to_params_with_types(
    values: &[Value],
    rust_types: &[&str],
) -> crate::Result<Vec<PostgreSQLParam>> {
    if rust_types.is_empty() {
        return values_to_params(values);
    }
    let mut params = Vec::with_capacity(values.len());
    for (idx, value) in values.iter().enumerate() {
        let rust_type = rust_types[idx % rust_types.len()];
        params.push(pg_value_to_param(value, Some(rust_type)));
    }
    Ok(params)
}

fn values_to_params_for_query(
    values: &[Value],
    rust_types: &[&str],
) -> crate::Result<Vec<PostgreSQLParam>> {
    if values.len() == rust_types.len() {
        values_to_params_with_types(values, rust_types)
    } else {
        values_to_params(values)
    }
}

fn values_to_params_auto_integer(values: &[Value]) -> Vec<PostgreSQLParam> {
    values
        .iter()
        .map(|value| match value {
            Value::Integer(value) if *value >= i32::MIN as i64 && *value <= i32::MAX as i64 => {
                Box::new(*value as i32) as PostgreSQLParam
            }
            Value::Integer(value) => Box::new(*value) as PostgreSQLParam,
            Value::Null => Box::new(None::<i32>) as PostgreSQLParam,
            value => pg_value_to_param(value, None),
        })
        .collect()
}

fn values_to_params_with_integer_hint(
    values: &[Value],
    rust_type: &'static str,
) -> Vec<PostgreSQLParam> {
    values
        .iter()
        .map(|value| match value {
            Value::Integer(_) | Value::Null => pg_value_to_param(value, Some(rust_type)),
            value => pg_value_to_param(value, None),
        })
        .collect()
}

/// 将 ormer Value 转换为 tokio-postgres 参数（旧版本，根据值大小选择类型）。
fn values_to_params(values: &[Value]) -> crate::Result<Vec<PostgreSQLParam>> {
    Ok(values
        .iter()
        .map(|value| pg_value_to_param(value, None))
        .collect())
}

/// Related 查询执行器（支持2表关联查询）
pub struct RelatedSelectExecutor<'a, T: Model, R: Model> {
    select: RelatedSelect<T, R>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, R)>,
}

/// SelectStream - 流式查询执行器 (PostgreSQL)
pub struct SelectStream<'a, T: Model> {
    select: Select<T>,
    conn: super::common::StreamConnection<'a>,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model> SelectExecutor<'a, T> {
    /// 创建流式查询执行器
    pub fn stream(self) -> SelectStream<'a, T> {
        SelectStream {
            select: self.select,
            conn: super::common::StreamConnection::PostgreSQL(self.client),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a, T: Model + 'static> SelectStream<'a, T> {
    /// 返回异步迭代器  
    pub async fn into_iter(self) -> crate::Result<SelectStreamIterator<'a, T>> {
        let param_rust_types = self.select.param_rust_types();
        let (sql, params) = self.select.try_to_sql_with_params(DbType::PostgreSQL)?;
        let pg_params = values_to_params_for_query(&params, &param_rust_types)?;
        let param_refs = pg_param_refs(&pg_params);

        // 从 StreamConnection 获取 client 引用
        let client = *self.conn.expect_postgresql();

        // 使用 query_raw 获取 RowStream
        let row_stream = client.query_raw(&sql, param_refs).trace().await?;

        Ok(SelectStreamIterator {
            _conn: self.conn,
            row_stream: Box::pin(row_stream),
            _marker: std::marker::PhantomData,
        })
    }
}

/// SelectStreamIterator - 流式查询迭代器 (PostgreSQL)
pub struct SelectStreamIterator<'a, T: Model> {
    _conn: super::common::StreamConnection<'a>,
    row_stream: std::pin::Pin<Box<tokio_postgres::RowStream>>,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T: Model + 'static> SelectStreamIterator<'a, T> {
    /// 获取下一行数据
    pub async fn next(&mut self) -> Option<crate::Result<T>> {
        use futures::StreamExt;

        match self.row_stream.next().await {
            Some(Ok(row)) => {
                // 解析行数据为 Model
                let mut data = HashMap::new();
                for (i, col_name) in T::COLUMNS.iter().enumerate() {
                    let ormer_value = match pg_model_value_from_row::<T>(&row, i, i) {
                        Ok(value) => value,
                        Err(err) => {
                            return Some(Err(err.context(format!("column '{}'", col_name))));
                        }
                    };

                    data.insert(col_name.to_string(), ormer_value);
                }
                let ormer_row = crate::model::Row::new(data);
                Some(T::from_row(&ormer_row))
            }
            Some(Err(e)) => Some(Err(crate::ormer_error!(
                "tokio_postgres::RowStream::next failed: {e}"
            ))),
            None => None,
        }
    }
}

impl_backend_related_executor_methods_with_lifetime!(
    RelatedSelectExecutor,
    client,
    &'a tokio_postgres::Client,
    RelatedSelect
);

impl<'a, T: Model, R: Model> RelatedSelectExecutor<'a, T, R> {
    pub async fn collect<C: FromIterator<T>>(self) -> crate::Result<C> {
        let results = self.collect_inner().trace().await?;
        Ok(results.into_iter().collect())
    }

    pub(crate) fn into_collect_future(self) -> RelatedCollectFuture<'a, T, R> {
        RelatedCollectFuture { executor: self }
    }

    async fn collect_inner(self) -> crate::Result<Vec<T>> {
        let param_rust_types = self.select.param_rust_types();
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let rows = pg_query_for_query(self.client, &sql, &params, &param_rust_types).await?;

        let mut results = Vec::new();
        for row in rows {
            let model = pg_decode_model_from_row::<T>(&row)?;
            results.push(model);
        }
        Ok(results)
    }
}

pub struct RelatedCollectFuture<'a, T: Model, R: Model> {
    executor: RelatedSelectExecutor<'a, T, R>,
}

impl<'a, T: Model + 'static + Send, R: Model + 'static + Send> std::future::IntoFuture
    for RelatedCollectFuture<'a, T, R>
{
    type Output = crate::Result<Vec<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

/// MultiTable 查询执行器（支持3表关联查询）
pub struct MultiTableSelectExecutor<'a, T: Model, R1: Model, R2: Model> {
    select: MultiTableSelect<T, R1, R2>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, R1, R2)>,
}

impl_backend_multi_table_executor_methods_with_lifetime!(
    MultiTableSelectExecutor,
    client,
    &'a tokio_postgres::Client,
    MultiTableSelect
);

impl<'a, T: Model, R1: Model, R2: Model> MultiTableSelectExecutor<'a, T, R1, R2> {
    async fn collect_inner(self) -> crate::Result<Vec<T>> {
        let param_rust_types = self.select.param_rust_types();
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let rows = pg_query_for_query(self.client, &sql, &params, &param_rust_types).await?;

        let mut results = Vec::new();
        for row in rows {
            let model = pg_decode_model_from_row::<T>(&row)?;
            results.push(model);
        }
        Ok(results)
    }
}

pub struct MultiTableCollectFuture<'a, T: Model, R1: Model, R2: Model> {
    executor: MultiTableSelectExecutor<'a, T, R1, R2>,
}

impl<'a, T: Model + 'static + Send, R1: Model + 'static + Send, R2: Model + 'static + Send>
    std::future::IntoFuture for MultiTableCollectFuture<'a, T, R1, R2>
{
    type Output = crate::Result<Vec<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

/// FourTable 查询执行器（支持4表关联查询）
pub struct FourTableSelectExecutor<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    select: FourTableSelect<T, R1, R2, R3>,
    client: &'a tokio_postgres::Client,
    _marker: PhantomData<(T, R1, R2, R3)>,
}

impl_backend_four_table_executor_methods_with_lifetime!(
    FourTableSelectExecutor,
    client,
    &'a tokio_postgres::Client,
    FourTableSelect
);

impl<'a, T: Model, R1: Model, R2: Model, R3: Model> FourTableSelectExecutor<'a, T, R1, R2, R3> {
    async fn collect_inner(self) -> crate::Result<Vec<T>> {
        let param_rust_types = self.select.param_rust_types();
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let rows = pg_query_for_query(self.client, &sql, &params, &param_rust_types).await?;

        let mut results = Vec::new();
        for row in rows {
            let model = pg_decode_model_from_row::<T>(&row)?;
            results.push(model);
        }
        Ok(results)
    }
}

pub struct FourTableCollectFuture<'a, T: Model, R1: Model, R2: Model, R3: Model> {
    executor: FourTableSelectExecutor<'a, T, R1, R2, R3>,
}

impl<
    'a,
    T: Model + 'static + Send,
    R1: Model + 'static + Send,
    R2: Model + 'static + Send,
    R3: Model + 'static + Send,
> std::future::IntoFuture for FourTableCollectFuture<'a, T, R1, R2, R3>
{
    type Output = crate::Result<Vec<T>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl_backend_join_executor_methods_with_lifetime!(
    LeftJoinedSelectExecutor,
    client,
    &'a tokio_postgres::Client,
    LeftJoinedSelect
);
impl_backend_join_executor_methods_with_lifetime!(
    InnerJoinedSelectExecutor,
    client,
    &'a tokio_postgres::Client,
    InnerJoinedSelect
);
impl_backend_join_executor_methods_with_lifetime!(
    RightJoinedSelectExecutor,
    client,
    &'a tokio_postgres::Client,
    RightJoinedSelect
);

/// LeftJoinedSelectExecutor 实现
impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    /// 克隆executor（保持相同的client引用）
    pub fn clone_with_client(&self) -> Self {
        Self {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(T, Option<J>)> + 'static>(
        &self,
    ) -> LeftJoinCollectFuture<'a, T, J> {
        LeftJoinCollectFuture {
            executor: self.clone_with_client(),
            _marker: PhantomData,
        }
    }
}

/// LEFT JOIN Collect future
pub struct LeftJoinCollectFuture<'a, T: Model, J: Model> {
    executor: LeftJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<(T, J)>,
}

impl<'a, T: Model + 'static + Send, J: Model + 'static + Send> std::future::IntoFuture
    for LeftJoinCollectFuture<'a, T, J>
{
    type Output = crate::Result<Vec<(T, Option<J>)>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> LeftJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(T, Option<J>)>>(self) -> crate::Result<C> {
        let param_rust_types = self.select.param_rust_types();
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let rows = pg_query_for_query(self.client, &sql, &params, &param_rust_types).await?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let ormer_value = pg_model_value_from_row::<T>(&row, i, i)?;
                t_data.insert(col_name.to_string(), ormer_value);
            }
            let t_model = T::from_row(&Row::new(t_data))?;

            // 尝试读取 J 的列
            let mut j_data = HashMap::new();
            let mut j_is_null = true;
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let ormer_value = pg_outer_join_model_value_from_row::<J>(&row, i, idx)?;
                if !matches!(ormer_value, crate::model::Value::Null) {
                    j_is_null = false;
                }
                j_data.insert(col_name.to_string(), ormer_value);
            }

            if j_is_null {
                results.push((t_model, None));
            } else {
                let j_model = J::from_row(&Row::new(j_data))?;
                results.push((t_model, Some(j_model)));
            }
        }

        Ok(results.into_iter().collect())
    }
}

/// InnerJoinedSelectExecutor 实现
impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    /// 克隆executor（保持相同的client引用）
    pub fn clone_with_client(&self) -> Self {
        Self {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(T, J)> + 'static>(&self) -> InnerJoinCollectFuture<'a, T, J> {
        InnerJoinCollectFuture {
            executor: self.clone_with_client(),
            _marker: PhantomData,
        }
    }
}

/// INNER JOIN Collect future
pub struct InnerJoinCollectFuture<'a, T: Model, J: Model> {
    executor: InnerJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<(T, J)>,
}

impl<'a, T: Model + 'static + Send, J: Model + 'static + Send> std::future::IntoFuture
    for InnerJoinCollectFuture<'a, T, J>
{
    type Output = crate::Result<Vec<(T, J)>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> InnerJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(T, J)>>(self) -> crate::Result<C> {
        let param_rust_types = self.select.param_rust_types();
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let rows = pg_query_for_query(self.client, &sql, &params, &param_rust_types).await?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let mut t_data = HashMap::new();
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let ormer_value = pg_model_value_from_row::<T>(&row, i, i)?;
                t_data.insert(col_name.to_string(), ormer_value);
            }
            let t_model = T::from_row(&Row::new(t_data))?;

            let mut j_data = HashMap::new();
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let ormer_value = pg_model_value_from_row::<J>(&row, i, idx)?;
                j_data.insert(col_name.to_string(), ormer_value);
            }

            let j_model = J::from_row(&Row::new(j_data))?;
            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

/// RightJoinedSelectExecutor 实现
impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    /// 克隆executor（保持相同的client引用）
    pub fn clone_with_client(&self) -> Self {
        Self {
            select: self.select.clone(),
            client: self.client,
            _marker: PhantomData,
        }
    }

    pub fn collect<C: FromIterator<(Option<T>, J)> + 'static>(
        &self,
    ) -> RightJoinCollectFuture<'a, T, J> {
        RightJoinCollectFuture {
            executor: self.clone_with_client(),
            _marker: PhantomData,
        }
    }
}

/// RIGHT JOIN Collect future
pub struct RightJoinCollectFuture<'a, T: Model, J: Model> {
    executor: RightJoinedSelectExecutor<'a, T, J>,
    _marker: PhantomData<(T, J)>,
}

/// Grouped Collect future（分组聚合查询）
pub struct GroupedCollectFuture<'a, T: Model, V, C> {
    executor: GroupedSelectExecutor<'a, T, V>,
    _marker: PhantomData<(T, V, C)>,
}

impl<'a, T: Model + 'static + Send, J: Model + 'static + Send> std::future::IntoFuture
    for RightJoinCollectFuture<'a, T, J>
{
    type Output = crate::Result<Vec<(Option<T>, J)>>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.executor.collect_inner().await })
    }
}

impl<'a, T: Model, J: Model> RightJoinedSelectExecutor<'a, T, J> {
    async fn collect_inner<C: FromIterator<(Option<T>, J)>>(self) -> crate::Result<C> {
        let param_rust_types = self.select.param_rust_types();
        let (sql, params) = self.select.to_sql_with_params(DbType::PostgreSQL);
        let rows = pg_query_for_query(self.client, &sql, &params, &param_rust_types).await?;

        let mut results = Vec::new();
        let t_col_count = T::COLUMNS.len();

        for row in rows {
            let mut t_data = HashMap::new();
            let mut t_is_null = true;
            for (i, col_name) in T::COLUMNS.iter().enumerate() {
                let ormer_value = pg_outer_join_model_value_from_row::<T>(&row, i, i)?;
                if !matches!(ormer_value, crate::model::Value::Null) {
                    t_is_null = false;
                }
                t_data.insert(col_name.to_string(), ormer_value);
            }

            let t_model = if t_is_null {
                None
            } else {
                Some(T::from_row(&Row::new(t_data))?)
            };

            let mut j_data = HashMap::new();
            for (i, col_name) in J::COLUMNS.iter().enumerate() {
                let idx = t_col_count + i;
                let ormer_value = pg_model_value_from_row::<J>(&row, i, idx)?;
                j_data.insert(col_name.to_string(), ormer_value);
            }

            let j_model = J::from_row(&Row::new(j_data))?;
            results.push((t_model, j_model));
        }

        Ok(results.into_iter().collect())
    }
}

/// 将 PostgreSQL 行中的值转换为 ormer Value
fn convert_postgres_value(
    row: &tokio_postgres::Row,
    index: usize,
) -> crate::Result<crate::model::Value> {
    use tokio_postgres::types::Type;

    let col_type = row.columns()[index].type_();

    if matches!(col_type.kind(), postgres_types::Kind::Enum(_)) {
        if let Ok(v) = row.try_get::<_, Option<PgEnumText>>(index) {
            return Ok(match v {
                Some(val) => crate::model::Value::Text(val.0),
                None => crate::model::Value::Null,
            });
        }
    }

    match col_type.name() {
        "_int4" => {
            if let Ok(v) = row.try_get::<_, Option<Vec<i32>>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::IntegerArray(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        "_int8" => {
            if let Ok(v) = row.try_get::<_, Option<Vec<Option<i64>>>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::NullableBigIntArray(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        _ => {}
    }

    if let Ok(v) = row.try_get::<_, Option<Vec<String>>>(index) {
        return Ok(match v {
            Some(val) => crate::model::Value::TextArray(val),
            None => crate::model::Value::Null,
        });
    }

    // 根据PostgreSQL类型选择正确的Rust类型
    match *col_type {
        // 整数类型 - 需要根据实际大小选择
        Type::INT2 => {
            if let Ok(v) = row.try_get::<_, Option<i16>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Integer(val as i64),
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::INT4 => {
            if let Ok(v) = row.try_get::<_, Option<i32>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Integer(val as i64),
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::INT8 => {
            if let Ok(v) = row.try_get::<_, Option<i64>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Integer(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::UUID => {
            if let Ok(v) = row.try_get::<_, Option<uuid::Uuid>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Uuid(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        // 文本类型
        Type::TEXT | Type::VARCHAR | Type::CHAR | Type::BPCHAR | Type::NAME => {
            if let Ok(v) = row.try_get::<_, Option<String>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Text(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        // 浮点类型
        Type::FLOAT4 => {
            if let Ok(v) = row.try_get::<_, Option<f32>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Real(val as f64),
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::FLOAT8 => {
            if let Ok(v) = row.try_get::<_, Option<f64>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Real(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::NUMERIC => {
            if let Ok(v) = row.try_get::<_, Option<PgNumericText>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::BigDecimal(val.0),
                    None => crate::model::Value::Null,
                });
            }
        }
        // 布尔类型
        Type::BOOL => {
            if let Ok(v) = row.try_get::<_, Option<bool>>(index) {
                return Ok(match v {
                    Some(true) => crate::model::Value::Integer(1),
                    Some(false) => crate::model::Value::Integer(0),
                    None => crate::model::Value::Null,
                });
            }
        }
        // 字节类型
        Type::BYTEA => {
            if let Ok(v) = row.try_get::<_, Option<Vec<u8>>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Bytes(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        // 日期时间类型
        Type::TIMESTAMP => {
            if let Ok(v) = row.try_get::<_, Option<chrono::NaiveDateTime>>(index) {
                return Ok(match v {
                    Some(val) => {
                        let utc = chrono::DateTime::from_naive_utc_and_offset(val, chrono::Utc);
                        crate::model::Value::DateTime(utc)
                    }
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::TIMESTAMPTZ => {
            if let Ok(v) = row.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::DateTime(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::DATE => {
            if let Ok(v) = row.try_get::<_, Option<chrono::NaiveDate>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Date(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        Type::TIME => {
            if let Ok(v) = row.try_get::<_, Option<chrono::NaiveTime>>(index) {
                return Ok(match v {
                    Some(val) => crate::model::Value::Time(val),
                    None => crate::model::Value::Null,
                });
            }
        }
        _ => {}
    }

    Err(crate::ormer_error!(format!(
        "Unsupported column type {:?} at index {}",
        col_type, index
    )))
}

impl<'a, T: Model, V> GroupedSelectExecutor<'a, T, V> {
    /// 执行查询并收集结果
    pub fn collect<C: FromIterator<V> + 'static>(&self) -> GroupedCollectFuture<'a, T, V, C>
    where
        T: 'static,
        V: crate::model::FromRowValues + 'static,
    {
        GroupedCollectFuture {
            executor: GroupedSelectExecutor {
                select: self.select.clone(),
                client: self.client,
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }

    pub fn as_model<R: Model>(self) -> crate::query::builder::DerivedSelect<R>
    where
        T: Send + Sync + 'static,
        V: Send + Sync + 'static,
    {
        self.select.as_model::<R>()
    }

    /// 添加 GROUP BY 字段
    pub fn group_by<F, G>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> G,
        G: crate::query::builder::GroupByColumns,
    {
        Self {
            select: self.select.group_by(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加 HAVING 条件
    pub fn having<F, W>(self, f: F) -> Self
    where
        F: FnOnce(<T as Model>::Where) -> W,
        W: Into<crate::query::builder::WhereExpr>,
    {
        Self {
            select: self.select.having(f),
            client: self.client,
            _marker: PhantomData,
        }
    }

    /// 添加 WHERE 条件（分组前过滤）
    pub fn filter<F, W>(self, f: F) -> Self
    where
        F: FnOnce(T::Where) -> W,
        W: Into<crate::query::builder::WhereExpr>,
    {
        Self {
            select: self.select.filter(f),
            client: self.client,
            _marker: PhantomData,
        }
    }
}

impl<
    'a,
    T: Model + 'static + Send,
    V: crate::model::FromRowValues + 'static + Send,
    C: FromIterator<V> + 'static,
> std::future::IntoFuture for GroupedCollectFuture<'a, T, V, C>
{
    type Output = crate::Result<C>;
    type IntoFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let (sql, params) = self
                .executor
                .select
                .try_to_sql_with_params(DbType::PostgreSQL)?;

            // 对于PostgreSQL,我们需要智能地转换参数类型
            // 如果SQL中包含::bigint(通常在HAVING子句中),使用i64
            // 否则使用i32
            let use_i64 = sql.contains("::bigint");

            let integer_rust_type = if use_i64 { "i64" } else { "i32" };
            let pg_params = values_to_params_with_integer_hint(&params, integer_rust_type);

            let param_refs = pg_param_refs(&pg_params);

            let rows = self
                .executor
                .client
                .query(&sql, &param_refs)
                .trace()
                .await?;

            let mut results = Vec::new();
            let column_count = self.executor.select.column_count();
            for row in rows {
                let v = pg_decode_row_values_from_row(&row, column_count)?;
                results.push(v);
            }

            Ok(results.into_iter().collect())
        })
    }
}
