use super::super::DbType;
use crate::model::{
    FromRowValues, Model, Row, TableRoute, Value, VersionSnapshotUpdate, quote_column_reference,
    quote_identifier, quote_qualified_identifier, routed_table_name_for_db,
};
use crate::query::filter::FilterExpr;
use crate::query::filter_formatter::FilterFormatter;
use crate::query::insert::{
    InsertAssignment, InsertConflict, InsertConflictAction, InsertConflictTarget, InsertValue,
};
use crate::query::update::{UpdateAssignment, UpdateExpr, UpdateValue};
use std::collections::{HashMap, HashSet};

pub fn placeholder(db_type: DbType, _param_idx: usize) -> String {
    match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => format!("${_param_idx}"),
        #[cfg(feature = "mssql")]
        DbType::MSSQL => format!("@P{_param_idx}"),
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => "?".to_string(),
        #[cfg(feature = "mysql")]
        DbType::MySQL => "?".to_string(),
    }
}

pub fn placeholder_list(db_type: DbType, start_idx: usize, count: usize) -> String {
    (0..count)
        .map(|offset| placeholder(db_type, start_idx + offset))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn quote_table_name<T: Model>(db_type: DbType) -> String {
    quote_qualified_identifier(db_type, T::table_name_for_db(db_type))
}

pub fn quote_routed_table_name<T: Model>(
    db_type: DbType,
    route: &TableRoute,
) -> crate::Result<String> {
    let table_name = routed_table_name_for_db(db_type, T::TABLE_NAME, route)?;
    Ok(quote_qualified_identifier(db_type, &table_name))
}

pub fn quote_column_list(db_type: DbType, columns: &[&str]) -> String {
    columns
        .iter()
        .map(|column| quote_identifier(db_type, column))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn quote_column_with_prefix(db_type: DbType, prefix: &str, column: &str) -> String {
    format!("{}.{}", prefix, quote_identifier(db_type, column))
}

pub fn quote_assignment(db_type: DbType, column: &str, value_sql: &str) -> String {
    format!("{} = {}", quote_identifier(db_type, column), value_sql)
}

#[derive(Debug, Clone)]
pub struct ModelUpdatePlan {
    pub sets: Vec<(String, Value)>,
    pub filters: Vec<FilterExpr>,
    pub version_update: Option<VersionSnapshotUpdate>,
}

pub type ModelUpdateBatch = Vec<ModelUpdatePlan>;

#[derive(Debug, Clone)]
pub struct ModelSqlStatement {
    pub sql: String,
    pub params: Vec<Value>,
    pub versioned: bool,
    pub version_update: Option<VersionSnapshotUpdate>,
    pub param_columns: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct InsertSqlStatement {
    pub sql: String,
    pub params: Vec<Value>,
    pub row_count: usize,
}

pub fn model_primary_key_filters<T: Model>(model: &T) -> Vec<FilterExpr> {
    T::primary_key_columns()
        .iter()
        .zip(model.primary_key_values())
        .map(|(col, val)| FilterExpr::Comparison {
            column: col.to_string(),
            operator: "=".to_string(),
            value: value_to_filter_value(&val),
        })
        .collect()
}

pub fn model_update_plan<T: Model>(
    model: &T,
    fields: Option<&[String]>,
) -> Option<ModelUpdatePlan> {
    let version_info = T::version_info();
    let old_version = version_info.map(|_| crate::model::model_version(model));
    let mut sets = match fields {
        Some(fields) => model.non_pk_field_values_for_columns(fields),
        None => model.non_pk_field_values(),
    }
    .into_iter()
    .filter(|(column, _)| {
        version_info
            .map(|info| *column != info.column)
            .unwrap_or(true)
    })
    .map(|(column, value)| (column.to_string(), value))
    .collect::<Vec<_>>();

    if let Some(info) = version_info {
        let next_version = old_version.unwrap_or(info.initial).saturating_add(1);
        sets.push((info.column.to_string(), Value::from(next_version)));
    }

    if sets.is_empty() {
        return None;
    }

    let mut filters = model_primary_key_filters(model);
    let version_update = if let Some(info) = version_info {
        let old_version = old_version.unwrap_or(info.initial);
        filters.push(FilterExpr::Comparison {
            column: info.column.to_string(),
            operator: "=".to_string(),
            value: value_to_filter_value(&Value::from(old_version)),
        });
        crate::model::version_snapshot_update(model, old_version)
    } else {
        None
    };

    Some(ModelUpdatePlan {
        sets,
        filters,
        version_update,
    })
}

pub fn model_delete_filters<T: Model>(model: &T) -> Vec<FilterExpr> {
    let mut filters = model_primary_key_filters(model);
    if let Some(info) = T::version_info() {
        let version = crate::model::model_version(model);
        filters.push(FilterExpr::Comparison {
            column: info.column.to_string(),
            operator: "=".to_string(),
            value: value_to_filter_value(&Value::from(version)),
        });
    }
    filters
}

pub fn push_filters_sql(
    db_type: DbType,
    sql: &mut String,
    params: &mut Vec<Value>,
    filters: &[FilterExpr],
) -> crate::Result<()> {
    if filters.is_empty() {
        return Ok(());
    }
    sql.push_str(" WHERE ");
    let mut param_idx = params.len() + 1;
    for (i, filter) in filters.iter().enumerate() {
        if i > 0 {
            sql.push_str(" AND ");
        }
        format_filter_with_params(filter, sql, &mut param_idx, params, db_type)?;
    }
    Ok(())
}

pub fn build_delete_sql<T: Model>(
    db_type: DbType,
    filters: &[FilterExpr],
) -> crate::Result<(String, Vec<Value>)> {
    let mut sql = format!("DELETE FROM {}", quote_table_name::<T>(db_type));
    let mut params = Vec::new();
    push_filters_sql(db_type, &mut sql, &mut params, filters)?;
    Ok((sql, params))
}

pub fn build_update_sql<T: Model>(
    db_type: DbType,
    sets: &[UpdateAssignment],
    filters: &[FilterExpr],
) -> crate::Result<(String, Vec<Value>)> {
    let mut sql = format!("UPDATE {} SET ", quote_table_name::<T>(db_type));
    let mut params = Vec::new();
    for (index, assignment) in sets.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&format_update_assignment(db_type, assignment, &mut params));
    }
    push_filters_sql(db_type, &mut sql, &mut params, filters)?;
    Ok((sql, params))
}

pub fn build_model_update_sql<T: Model>(
    db_type: DbType,
    plan: &ModelUpdatePlan,
) -> crate::Result<ModelSqlStatement> {
    let mut sql = format!("UPDATE {} SET ", quote_table_name::<T>(db_type));
    let mut params = Vec::new();
    for (index, (column, value)) in plan.sets.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&quote_assignment(
            db_type,
            column,
            &placeholder(db_type, params.len() + 1),
        ));
        params.push(value.clone());
    }
    push_filters_sql(db_type, &mut sql, &mut params, &plan.filters)?;
    Ok(ModelSqlStatement {
        sql,
        params,
        versioned: plan.version_update.is_some(),
        version_update: plan.version_update.clone(),
        param_columns: None,
    })
}

pub fn bind_param_limit(db_type: DbType) -> usize {
    match db_type {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => 999,
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => 65_535,
        #[cfg(feature = "mysql")]
        DbType::MySQL => 65_535,
        #[cfg(feature = "mssql")]
        DbType::MSSQL => 2_100,
    }
}

fn value_key(value: &Value) -> String {
    match value {
        Value::Integer(v) => format!("i:{v}"),
        Value::BigInt(v) => format!("b:{v}"),
        Value::Duration(v) => format!("du:{v:?}"),
        Value::Text(v) => format!("t:{v}"),
        Value::TextArray(v) => format!("ta:{v:?}"),
        Value::Real(v) => format!("r:{v}"),
        Value::Decimal(v) => format!("de:{v}"),
        Value::BigDecimal(v) => format!("bd:{v}"),
        Value::Boolean(v) => format!("bo:{v}"),
        Value::Bytes(v) => format!("x:{v:?}"),
        Value::IntegerArray(v) => format!("ia:{v:?}"),
        Value::BigIntArray(v) => format!("ba:{v:?}"),
        Value::NullableBigIntArray(v) => format!("nba:{v:?}"),
        Value::DateTime(v) => format!("dt:{v}"),
        Value::Date(v) => format!("d:{v}"),
        Value::Time(v) => format!("ti:{v}"),
        Value::Json(v) => format!("j:{v}"),
        Value::Uuid(v) => format!("u:{v}"),
        Value::Null => "n:".to_string(),
    }
}

fn extract_model_update_pk_values<T: Model>(plan: &ModelUpdatePlan) -> Option<Vec<Value>> {
    let pk_columns = T::primary_key_columns();
    if pk_columns.is_empty() || plan.filters.len() != pk_columns.len() {
        return None;
    }

    let mut values = Vec::with_capacity(pk_columns.len());
    for pk in pk_columns {
        let value = plan.filters.iter().find_map(|filter| match filter {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } if column == pk && operator == "=" => Some(value.clone()),
            _ => None,
        })?;
        values.push(value);
    }
    Some(values)
}

fn model_update_set_columns(plans: &[ModelUpdatePlan]) -> Option<Vec<String>> {
    let first = plans.first()?;
    let columns = first
        .sets
        .iter()
        .map(|(column, _)| column.clone())
        .collect::<Vec<_>>();

    if columns.is_empty() {
        return None;
    }

    plans
        .iter()
        .all(|plan| {
            plan.version_update.is_none()
                && plan.sets.len() == columns.len()
                && plan
                    .sets
                    .iter()
                    .zip(&columns)
                    .all(|((column, _), expected)| column == expected)
        })
        .then_some(columns)
}

fn model_update_pk_values<T: Model>(plans: &[ModelUpdatePlan]) -> Option<Vec<Vec<Value>>> {
    let mut seen = HashSet::new();
    let mut values = Vec::with_capacity(plans.len());
    for plan in plans {
        let pk_values = extract_model_update_pk_values::<T>(plan)?;
        let key = pk_values
            .iter()
            .map(value_key)
            .collect::<Vec<_>>()
            .join("|");
        if !seen.insert(key) {
            return None;
        }
        values.push(pk_values);
    }
    Some(values)
}

fn bulk_update_params_per_row(db_type: DbType, pk_count: usize, set_count: usize) -> usize {
    #[allow(unreachable_patterns)]
    match db_type {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => set_count * (pk_count + 1) + pk_count,
        _ => pk_count + set_count,
    }
}

fn bulk_update_rows_per_statement(db_type: DbType, pk_count: usize, set_count: usize) -> usize {
    let params_per_row = bulk_update_params_per_row(db_type, pk_count, set_count).max(1);
    (bind_param_limit(db_type) / params_per_row).max(1)
}

fn push_pk_match_sql(
    db_type: DbType,
    sql: &mut String,
    pk_columns: &[&'static str],
    pk_values: &[Value],
    params: &mut Vec<Value>,
) {
    if pk_columns.len() > 1 {
        sql.push('(');
    }
    for (index, pk) in pk_columns.iter().enumerate() {
        if index > 0 {
            sql.push_str(" AND ");
        }
        sql.push_str(&format!(
            "{} = {}",
            quote_identifier(db_type, pk),
            placeholder(db_type, params.len() + 1)
        ));
        params.push(pk_values[index].clone());
    }
    if pk_columns.len() > 1 {
        sql.push(')');
    }
}

fn plan_set_value<'a>(plan: &'a ModelUpdatePlan, column: &str) -> Option<&'a Value> {
    plan.sets
        .iter()
        .find_map(|(set_column, value)| (set_column == column).then_some(value))
}

#[cfg(feature = "sqlite")]
fn build_sqlite_bulk_model_update_sql<T: Model>(
    plans: &[ModelUpdatePlan],
    pk_values: &[Vec<Value>],
    set_columns: &[String],
) -> crate::Result<ModelSqlStatement> {
    let db_type = DbType::Sqlite;
    let pk_columns = T::primary_key_columns();
    let mut sql = format!("UPDATE {} SET ", quote_table_name::<T>(db_type));
    let mut params = Vec::new();
    let mut param_columns = Vec::new();

    for (set_index, column) in set_columns.iter().enumerate() {
        if set_index > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&format!("{} = CASE ", quote_identifier(db_type, column)));
        for (plan, pk_values) in plans.iter().zip(pk_values) {
            sql.push_str("WHEN ");
            push_pk_match_sql(db_type, &mut sql, pk_columns, pk_values, &mut params);
            param_columns.extend(pk_columns.iter().map(|column| (*column).to_string()));
            sql.push_str(" THEN ");
            sql.push_str(&placeholder(db_type, params.len() + 1));
            let value = plan_set_value(plan, column).ok_or_else(|| {
                crate::ormer_error!("Missing bulk update value for column {column}")
            })?;
            params.push(value.clone());
            param_columns.push(column.clone());
            sql.push(' ');
        }
        sql.push_str(&format!("ELSE {} END", quote_identifier(db_type, column)));
    }

    sql.push_str(" WHERE ");
    for (index, pk_values) in pk_values.iter().enumerate() {
        if index > 0 {
            sql.push_str(" OR ");
        }
        push_pk_match_sql(db_type, &mut sql, pk_columns, pk_values, &mut params);
        param_columns.extend(pk_columns.iter().map(|column| (*column).to_string()));
    }

    Ok(ModelSqlStatement {
        sql,
        params,
        versioned: false,
        version_update: None,
        param_columns: Some(param_columns),
    })
}

#[cfg(any(feature = "postgresql", feature = "mysql", feature = "mssql"))]
fn bulk_source_columns<'a>(
    pk_columns: &'a [&'static str],
    set_columns: &'a [String],
) -> Vec<&'a str> {
    pk_columns
        .iter()
        .map(|column| *column)
        .chain(set_columns.iter().map(String::as_str))
        .collect()
}

#[cfg(any(feature = "postgresql", feature = "mysql", feature = "mssql"))]
fn push_bulk_source_row_values<T: Model>(
    params: &mut Vec<Value>,
    param_columns: &mut Vec<String>,
    plan: &ModelUpdatePlan,
    pk_values: &[Value],
    set_columns: &[String],
) -> crate::Result<()> {
    for (pk, value) in T::primary_key_columns().iter().zip(pk_values) {
        params.push(value.clone());
        param_columns.push((*pk).to_string());
    }
    for column in set_columns {
        let value = plan_set_value(plan, column)
            .ok_or_else(|| crate::ormer_error!("Missing bulk update value for column {column}"))?;
        params.push(value.clone());
        param_columns.push(column.clone());
    }
    Ok(())
}

#[cfg(feature = "postgresql")]
fn postgres_bulk_source_cast_type<T: Model>(column: &str) -> crate::Result<String> {
    let schema = T::COLUMN_SCHEMA
        .iter()
        .find(|schema| schema.name == column)
        .ok_or_else(|| crate::ormer_error!("Unknown bulk update column {column}"))?;

    if let Some(db_value_type) = schema.db_value_type {
        return Ok(db_value_type(DbType::PostgreSQL).to_string());
    }

    Ok(DbType::PostgreSQL.sql_type(
        schema.data_type.unwrap_or(schema.rust_type),
        false,
        false,
        true,
        schema.enum_variants,
    ))
}

#[cfg(feature = "postgresql")]
fn postgres_casted_placeholder_list<T: Model>(
    start_idx: usize,
    source_columns: &[&str],
) -> crate::Result<String> {
    source_columns
        .iter()
        .enumerate()
        .map(|(offset, column)| {
            let cast_type = postgres_bulk_source_cast_type::<T>(column)?;
            Ok(format!(
                "CAST({} AS {cast_type})",
                placeholder(DbType::PostgreSQL, start_idx + offset)
            ))
        })
        .collect::<crate::Result<Vec<_>>>()
        .map(|parts| parts.join(", "))
}

#[cfg(any(feature = "postgresql", feature = "mssql"))]
fn build_values_source_bulk_model_update_sql<T: Model>(
    db_type: DbType,
    plans: &[ModelUpdatePlan],
    pk_values: &[Vec<Value>],
    set_columns: &[String],
) -> crate::Result<ModelSqlStatement> {
    let pk_columns = T::primary_key_columns();
    let source_columns = bulk_source_columns(pk_columns, set_columns);
    let source_column_list = quote_column_list(db_type, &source_columns);
    let source_width = source_columns.len();
    let mut params = Vec::new();
    let mut param_columns = Vec::new();

    let mut values_sql = String::new();
    for (index, (plan, pk_values)) in plans.iter().zip(pk_values).enumerate() {
        if index > 0 {
            values_sql.push_str(", ");
        }
        values_sql.push('(');
        #[cfg(feature = "postgresql")]
        if matches!(db_type, DbType::PostgreSQL) {
            values_sql.push_str(&postgres_casted_placeholder_list::<T>(
                params.len() + 1,
                &source_columns,
            )?);
        } else {
            values_sql.push_str(&placeholder_list(db_type, params.len() + 1, source_width));
        }
        #[cfg(not(feature = "postgresql"))]
        values_sql.push_str(&placeholder_list(db_type, params.len() + 1, source_width));
        values_sql.push(')');
        push_bulk_source_row_values::<T>(
            &mut params,
            &mut param_columns,
            plan,
            pk_values,
            set_columns,
        )?;
    }

    let table = quote_table_name::<T>(db_type);
    let sql = match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            let assignments = set_columns
                .iter()
                .map(|column| {
                    quote_assignment(
                        db_type,
                        column,
                        &quote_column_with_prefix(db_type, "source", column),
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let predicates = pk_columns
                .iter()
                .map(|pk| {
                    format!(
                        "{} = {}",
                        quote_column_with_prefix(db_type, "target", pk),
                        quote_column_with_prefix(db_type, "source", pk)
                    )
                })
                .collect::<Vec<_>>()
                .join(" AND ");
            format!(
                "UPDATE {table} AS target SET {assignments} FROM (VALUES {values_sql}) AS source ({source_column_list}) WHERE {predicates}"
            )
        }
        #[cfg(feature = "mssql")]
        DbType::MSSQL => {
            let assignments = set_columns
                .iter()
                .map(|column| {
                    format!(
                        "{} = {}",
                        quote_column_with_prefix(db_type, "target", column),
                        quote_column_with_prefix(db_type, "source", column)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let predicates = pk_columns
                .iter()
                .map(|pk| {
                    format!(
                        "{} = {}",
                        quote_column_with_prefix(db_type, "target", pk),
                        quote_column_with_prefix(db_type, "source", pk)
                    )
                })
                .collect::<Vec<_>>()
                .join(" AND ");
            format!(
                "UPDATE target SET {assignments} FROM {table} AS target JOIN (VALUES {values_sql}) AS source ({source_column_list}) ON {predicates}"
            )
        }
        _ => {
            return Err(crate::ormer_error!(
                "VALUES source bulk update is not supported for this database"
            ));
        }
    };

    Ok(ModelSqlStatement {
        sql,
        params,
        versioned: false,
        version_update: None,
        param_columns: Some(param_columns),
    })
}

#[cfg(feature = "mysql")]
fn build_mysql_bulk_model_update_sql<T: Model>(
    plans: &[ModelUpdatePlan],
    pk_values: &[Vec<Value>],
    set_columns: &[String],
) -> crate::Result<ModelSqlStatement> {
    let db_type = DbType::MySQL;
    let pk_columns = T::primary_key_columns();
    let source_columns = bulk_source_columns(pk_columns, set_columns);
    let source_width = source_columns.len();
    let mut params = Vec::new();
    let mut param_columns = Vec::new();
    let mut source_sql = String::new();

    for (index, (plan, pk_values)) in plans.iter().zip(pk_values).enumerate() {
        if index > 0 {
            source_sql.push_str(" UNION ALL ");
        }
        source_sql.push_str("SELECT ");
        for (column_index, column) in source_columns.iter().enumerate() {
            if column_index > 0 {
                source_sql.push_str(", ");
            }
            source_sql.push_str(&placeholder(db_type, params.len() + column_index + 1));
            if index == 0 {
                source_sql.push_str(" AS ");
                source_sql.push_str(&quote_identifier(db_type, column));
            }
        }
        push_bulk_source_row_values::<T>(
            &mut params,
            &mut param_columns,
            plan,
            pk_values,
            set_columns,
        )?;
        debug_assert_eq!(params.len(), (index + 1) * source_width);
    }

    let assignments = set_columns
        .iter()
        .map(|column| {
            format!(
                "{} = {}",
                quote_column_with_prefix(db_type, "target", column),
                quote_column_with_prefix(db_type, "source", column)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let predicates = pk_columns
        .iter()
        .map(|pk| {
            format!(
                "{} = {}",
                quote_column_with_prefix(db_type, "target", pk),
                quote_column_with_prefix(db_type, "source", pk)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");

    Ok(ModelSqlStatement {
        sql: format!(
            "UPDATE {} AS target JOIN ({source_sql}) AS source ON {predicates} SET {assignments}",
            quote_table_name::<T>(db_type)
        ),
        params,
        versioned: false,
        version_update: None,
        param_columns: Some(param_columns),
    })
}

fn build_bulk_model_update_sql<T: Model>(
    db_type: DbType,
    plans: &[ModelUpdatePlan],
    pk_values: &[Vec<Value>],
    set_columns: &[String],
) -> crate::Result<ModelSqlStatement> {
    match db_type {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => build_sqlite_bulk_model_update_sql::<T>(plans, pk_values, set_columns),
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            build_values_source_bulk_model_update_sql::<T>(db_type, plans, pk_values, set_columns)
        }
        #[cfg(feature = "mysql")]
        DbType::MySQL => build_mysql_bulk_model_update_sql::<T>(plans, pk_values, set_columns),
        #[cfg(feature = "mssql")]
        DbType::MSSQL => {
            build_values_source_bulk_model_update_sql::<T>(db_type, plans, pk_values, set_columns)
        }
    }
}

pub fn build_bulk_model_update_statements<T: Model>(
    db_type: DbType,
    plans: &[ModelUpdatePlan],
) -> crate::Result<Option<Vec<ModelSqlStatement>>> {
    if plans.len() <= 1 {
        return Ok(None);
    }

    let set_columns = match model_update_set_columns(plans) {
        Some(columns) => columns,
        None => return Ok(None),
    };
    let pk_values = match model_update_pk_values::<T>(plans) {
        Some(values) => values,
        None => return Ok(None),
    };
    let pk_count = T::primary_key_columns().len();
    let rows_per_statement =
        bulk_update_rows_per_statement(db_type, pk_count, set_columns.len()).max(1);
    if rows_per_statement <= 1 {
        return Ok(None);
    }

    let mut statements = Vec::new();
    let mut start = 0;
    while start < plans.len() {
        let end = (start + rows_per_statement).min(plans.len());
        statements.push(build_bulk_model_update_sql::<T>(
            db_type,
            &plans[start..end],
            &pk_values[start..end],
            &set_columns,
        )?);
        start = end;
    }

    Ok(Some(statements))
}

pub fn optimistic_lock_conflict<T: Model>() -> crate::OrmerError {
    if let Some(info) = T::version_info() {
        crate::OrmerError::optimistic_lock(T::TABLE_NAME, info.column)
    } else {
        crate::ormer_error!("Optimistic lock conflict on {}", T::TABLE_NAME)
    }
}

pub fn format_update_assignment(
    db_type: DbType,
    assignment: &UpdateAssignment,
    params: &mut Vec<Value>,
) -> String {
    let value_sql = format_update_value(db_type, &assignment.value, params);
    quote_assignment(db_type, &assignment.column, &value_sql)
}

fn format_update_value(db_type: DbType, value: &UpdateValue, params: &mut Vec<Value>) -> String {
    match value {
        UpdateValue::Literal(value) => {
            params.push(value.clone());
            placeholder(db_type, params.len())
        }
        UpdateValue::Expr(expr) => format_update_expr(db_type, expr, params),
    }
}

fn format_update_expr(db_type: DbType, expr: &UpdateExpr, params: &mut Vec<Value>) -> String {
    match expr {
        UpdateExpr::Column(column) => quote_column_reference(db_type, column),
        UpdateExpr::IncomingColumn(column) => quote_column_reference(db_type, column),
        UpdateExpr::Value(value) => {
            params.push(value.clone());
            placeholder(db_type, params.len())
        }
        UpdateExpr::Binary { left, op, right } => format!(
            "{} {} {}",
            format_update_expr(db_type, left, params),
            op.sql(),
            format_update_expr(db_type, right, params)
        ),
        UpdateExpr::Sql(expr) => {
            let mut param_idx = params.len() as i32 + 1;
            expr.to_sql(db_type, &mut param_idx, params, None)
        }
    }
}

fn incoming_column_sql(db_type: DbType, column: &str) -> String {
    match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => quote_column_with_prefix(db_type, "EXCLUDED", column),
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => quote_column_with_prefix(db_type, "excluded", column),
        #[cfg(feature = "mysql")]
        DbType::MySQL => format!("VALUES({})", quote_identifier(db_type, column)),
        #[cfg(feature = "mssql")]
        DbType::MSSQL => quote_column_with_prefix(db_type, "source", column),
    }
}

fn format_upsert_update_value(
    db_type: DbType,
    value: &UpdateValue,
    params: &mut Vec<Value>,
) -> String {
    match value {
        UpdateValue::Literal(value) => {
            params.push(value.clone());
            placeholder(db_type, params.len())
        }
        UpdateValue::Expr(expr) => format_upsert_update_expr(db_type, expr, params),
    }
}

fn format_upsert_update_expr(
    db_type: DbType,
    expr: &UpdateExpr,
    params: &mut Vec<Value>,
) -> String {
    match expr {
        UpdateExpr::Column(column) => quote_column_reference(db_type, column),
        UpdateExpr::IncomingColumn(column) => incoming_column_sql(db_type, column),
        UpdateExpr::Value(value) => {
            params.push(value.clone());
            placeholder(db_type, params.len())
        }
        UpdateExpr::Binary { left, op, right } => format!(
            "{} {} {}",
            format_upsert_update_expr(db_type, left, params),
            op.sql(),
            format_upsert_update_expr(db_type, right, params)
        ),
        UpdateExpr::Sql(expr) => {
            let mut param_idx = params.len() as i32 + 1;
            expr.to_sql(db_type, &mut param_idx, params, None)
        }
    }
}

pub fn format_upsert_update_assignment(
    db_type: DbType,
    assignment: &UpdateAssignment,
    params: &mut Vec<Value>,
) -> String {
    let value_sql = format_upsert_update_value(db_type, &assignment.value, params);
    quote_assignment(db_type, &assignment.column, &value_sql)
}

pub fn quote_postgres_excluded_assignment(db_type: DbType, column: &str) -> String {
    quote_assignment(
        db_type,
        column,
        &quote_column_with_prefix(db_type, "EXCLUDED", column),
    )
}

pub fn quote_mysql_values_assignment(db_type: DbType, column: &str) -> String {
    quote_assignment(
        db_type,
        column,
        &format!("VALUES({})", quote_identifier(db_type, column)),
    )
}

pub fn sql_type_with_nullability(base_type: &str, is_nullable: bool) -> String {
    format!("{base_type}{}", if is_nullable { "" } else { " NOT NULL" })
}

/// 通用过滤器格式化函数（不包含参数值，用于 DELETE）
pub fn format_filter(
    filter: &FilterExpr,
    sql: &mut String,
    param_idx: &mut i32,
    db_type: DbType,
) -> crate::Result<()> {
    let mut params = Vec::new();
    sql.push_str(&FilterFormatter::new(db_type).format(filter, param_idx, &mut params));
    Ok(())
}

/// 通用过滤器格式化函数并收集参数（用于 UPDATE/SELECT）
pub fn format_filter_with_params(
    filter: &FilterExpr,
    sql: &mut String,
    param_idx: &mut usize,
    params: &mut Vec<Value>,
    db_type: DbType,
) -> crate::Result<()> {
    let mut next_param_idx = *param_idx as i32;
    sql.push_str(&FilterFormatter::new(db_type).format(filter, &mut next_param_idx, params));
    *param_idx = next_param_idx as usize;
    Ok(())
}

/// 通用行数据提取函数 - 从数据库行中提取模型数据
pub fn extract_model_from_row<T: Model>(row_data: &HashMap<String, Value>) -> crate::Result<T> {
    let row = Row::new(row_data.clone());
    T::from_row(&row)
}

pub fn decode_model_from_indexed_values<T, F>(offset: usize, mut value_at: F) -> crate::Result<T>
where
    T: Model,
    F: FnMut(usize) -> crate::Result<Value>,
{
    let mut data = HashMap::new();
    for (i, col_name) in T::columns().iter().enumerate() {
        data.insert(col_name.to_string(), value_at(offset + i)?);
    }

    T::from_row(&Row::new(data))
}

pub fn decode_optional_model_from_indexed_values<T, F>(
    offset: usize,
    mut value_at: F,
) -> crate::Result<Option<T>>
where
    T: Model,
    F: FnMut(usize) -> crate::Result<Value>,
{
    let mut data = HashMap::new();
    let mut is_null = true;
    for (i, col_name) in T::columns().iter().enumerate() {
        let value = value_at(offset + i)?;
        if !matches!(value, Value::Null) {
            is_null = false;
        }
        data.insert(col_name.to_string(), value);
    }

    if is_null {
        Ok(None)
    } else {
        Ok(Some(T::from_row(&Row::new(data))?))
    }
}

pub fn decode_row_values_from_indexed_values<V, F>(
    column_count: usize,
    mut value_at: F,
) -> crate::Result<V>
where
    V: FromRowValues,
    F: FnMut(usize) -> crate::Result<Value>,
{
    let mut values = Vec::with_capacity(column_count);
    for i in 0..column_count {
        values.push(value_at(i)?);
    }

    V::from_row_values(&values)
}

/// 通用列值转换助手 - 根据 rust_type 转换数据库值到 ormer Value
#[derive(Clone, Copy)]
enum ColumnValueMode<'a> {
    Default,
    Strict { column_name: &'a str },
}

#[allow(clippy::too_many_arguments)]
fn parse_column_value_options(
    rust_type: &str,
    is_nullable: bool,
    int: Option<i64>,
    string: Option<String>,
    real: Option<f64>,
    boolean: Option<i8>,
    bytes: Option<Vec<u8>>,
    datetime: Option<chrono::DateTime<chrono::Utc>>,
    mode: ColumnValueMode<'_>,
) -> crate::Result<Value> {
    fn decimal_string(
        string: Option<String>,
        int: Option<i64>,
        real: Option<f64>,
    ) -> Option<String> {
        string
            .or_else(|| int.map(|value| value.to_string()))
            .or_else(|| real.map(|value| value.to_string()))
    }

    if is_nullable {
        match rust_type {
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => match int {
                Some(val) => Ok(Value::Integer(val)),
                None => Ok(Value::Null),
            },
            "String" => match string {
                Some(val) => Ok(Value::Text(val)),
                None => Ok(Value::Null),
            },
            "f32" | "f64" => match real {
                Some(val) => Ok(Value::Real(val)),
                None => Ok(Value::Null),
            },
            "Decimal" | "rust_decimal::Decimal" => match decimal_string(string, int, real) {
                Some(val) => Ok(Value::Decimal(val)),
                None => Ok(Value::Null),
            },
            "BigDecimal" | "bigdecimal::BigDecimal" => match decimal_string(string, int, real) {
                Some(val) => Ok(Value::BigDecimal(val)),
                None => Ok(Value::Null),
            },
            "bool" => match boolean {
                Some(1) => Ok(Value::Boolean(true)),
                Some(0) => Ok(Value::Boolean(false)),
                _ => Ok(Value::Null),
            },
            "Vec<u8>" | "&[u8]" => match bytes {
                Some(val) => Ok(Value::Bytes(val)),
                None => Ok(Value::Null),
            },
            "DateTime"
            | "chrono::DateTime"
            | "chrono::DateTime<chrono::Utc>"
            | "NaiveDateTime"
            | "chrono::NaiveDateTime" => match datetime {
                Some(val) => Ok(Value::DateTime(val)),
                None => Ok(Value::Null),
            },
            "NaiveDate" | "chrono::NaiveDate" => match string {
                Some(val) => Ok(Value::Date(chrono::NaiveDate::parse_from_str(
                    &val, "%Y-%m-%d",
                )?)),
                None => Ok(Value::Null),
            },
            "NaiveTime" | "chrono::NaiveTime" => match string {
                Some(val) => Ok(Value::Time(chrono::NaiveTime::parse_from_str(
                    &val,
                    "%H:%M:%S%.f",
                )?)),
                None => Ok(Value::Null),
            },
            _ => Err(crate::ormer_error!(
                "Unsupported nullable column type: {rust_type}"
            )),
        }
    } else {
        match (rust_type, mode) {
            (
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64",
                ColumnValueMode::Default,
            ) => Ok(Value::Integer(int.unwrap_or(0))),
            (
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64",
                ColumnValueMode::Strict { column_name },
            ) => int.map(Value::Integer).ok_or_else(|| {
                crate::ormer_error!(
                    "Failed to parse non-nullable column '{}' (expected integer type)",
                    column_name
                )
            }),
            ("String", ColumnValueMode::Default) => Ok(Value::Text(string.unwrap_or_default())),
            ("String", ColumnValueMode::Strict { column_name }) => {
                string.map(Value::Text).ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected String type)",
                        column_name
                    )
                })
            }
            ("f32" | "f64", ColumnValueMode::Default) => Ok(Value::Real(real.unwrap_or(0.0))),
            ("f32" | "f64", ColumnValueMode::Strict { column_name }) => {
                real.map(Value::Real).ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected float type)",
                        column_name
                    )
                })
            }
            ("Decimal" | "rust_decimal::Decimal", ColumnValueMode::Default) => Ok(Value::Decimal(
                decimal_string(string, int, real).unwrap_or_else(|| "0".to_string()),
            )),
            ("Decimal" | "rust_decimal::Decimal", ColumnValueMode::Strict { column_name }) => {
                decimal_string(string, int, real)
                    .map(Value::Decimal)
                    .ok_or_else(|| {
                        crate::ormer_error!(
                            "Failed to parse non-nullable column '{}' (expected Decimal type)",
                            column_name
                        )
                    })
            }
            ("BigDecimal" | "bigdecimal::BigDecimal", ColumnValueMode::Default) => {
                Ok(Value::BigDecimal(
                    decimal_string(string, int, real).unwrap_or_else(|| "0".to_string()),
                ))
            }
            ("BigDecimal" | "bigdecimal::BigDecimal", ColumnValueMode::Strict { column_name }) => {
                decimal_string(string, int, real)
                    .map(Value::BigDecimal)
                    .ok_or_else(|| {
                        crate::ormer_error!(
                            "Failed to parse non-nullable column '{}' (expected BigDecimal type)",
                            column_name
                        )
                    })
            }
            ("bool", ColumnValueMode::Default) => Ok(Value::Boolean(boolean.unwrap_or(0) == 1)),
            ("bool", ColumnValueMode::Strict { column_name }) => boolean
                .map(|value| Value::Boolean(value == 1))
                .ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected bool type)",
                        column_name
                    )
                }),
            ("Vec<u8>" | "&[u8]", ColumnValueMode::Default) => {
                Ok(Value::Bytes(bytes.unwrap_or_default()))
            }
            ("Vec<u8>" | "&[u8]", ColumnValueMode::Strict { column_name }) => {
                bytes.map(Value::Bytes).ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected Vec<u8> type)",
                        column_name
                    )
                })
            }
            ("Duration" | "std::time::Duration", ColumnValueMode::Default) => Ok(Value::Duration(
                std::time::Duration::from_micros(int.unwrap_or(0).max(0) as u64),
            )),
            ("Duration" | "std::time::Duration", ColumnValueMode::Strict { .. }) => {
                Err(crate::ormer_error!("Unsupported column type: {rust_type}"))
            }
            (
                "DateTime"
                | "chrono::DateTime"
                | "chrono::DateTime<chrono::Utc>"
                | "NaiveDateTime"
                | "chrono::NaiveDateTime",
                ColumnValueMode::Default,
            ) => Ok(Value::DateTime(
                datetime.unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH),
            )),
            (
                "DateTime"
                | "chrono::DateTime"
                | "chrono::DateTime<chrono::Utc>"
                | "NaiveDateTime"
                | "chrono::NaiveDateTime",
                ColumnValueMode::Strict { column_name },
            ) => datetime.map(Value::DateTime).ok_or_else(|| {
                crate::ormer_error!(
                    "Failed to parse non-nullable column '{}' (expected DateTime type)",
                    column_name
                )
            }),
            ("NaiveDate" | "chrono::NaiveDate", ColumnValueMode::Default) => Ok(Value::Date(
                string
                    .and_then(|value| chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok())
                    .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()),
            )),
            ("NaiveDate" | "chrono::NaiveDate", ColumnValueMode::Strict { column_name }) => {
                let value = string.ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected date type)",
                        column_name
                    )
                })?;
                Ok(Value::Date(chrono::NaiveDate::parse_from_str(
                    &value, "%Y-%m-%d",
                )?))
            }
            ("NaiveTime" | "chrono::NaiveTime", ColumnValueMode::Default) => Ok(Value::Time(
                string
                    .and_then(|value| chrono::NaiveTime::parse_from_str(&value, "%H:%M:%S%.f").ok())
                    .unwrap_or_else(|| chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()),
            )),
            ("NaiveTime" | "chrono::NaiveTime", ColumnValueMode::Strict { column_name }) => {
                let value = string.ok_or_else(|| {
                    crate::ormer_error!(
                        "Failed to parse non-nullable column '{}' (expected time type)",
                        column_name
                    )
                })?;
                Ok(Value::Time(chrono::NaiveTime::parse_from_str(
                    &value,
                    "%H:%M:%S%.f",
                )?))
            }
            _ => Err(crate::ormer_error!("Unsupported column type: {rust_type}")),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn convert_column_value(
    rust_type: &str,
    is_nullable: bool,
    get_int: impl FnOnce() -> Option<i64>,
    get_string: impl FnOnce() -> Option<String>,
    get_real: impl FnOnce() -> Option<f64>,
    get_bool: impl FnOnce() -> Option<i8>,
    get_bytes: impl FnOnce() -> Option<Vec<u8>>,
    get_datetime: impl FnOnce() -> Option<chrono::DateTime<chrono::Utc>>,
) -> crate::Result<Value> {
    parse_column_value_options(
        rust_type,
        is_nullable,
        get_int(),
        get_string(),
        get_real(),
        get_bool(),
        get_bytes(),
        get_datetime(),
        ColumnValueMode::Default,
    )
}

/// 将 model::Value 转换为 filter::Value
pub fn value_to_filter_value(val: &Value) -> crate::query::filter::Value {
    val.clone()
}

fn downcast_auto_increment_key<K: 'static, T: 'static>(value: T) -> K {
    let boxed: Box<dyn std::any::Any> = Box::new(value);
    match boxed.downcast::<K>() {
        Ok(value) => *value,
        Err(_) => unreachable!("auto-increment key type checked before downcast"),
    }
}

/// 将数据库返回的自增 ID 转换为模型指定的 AutoIncrementKeyType。
pub fn convert_auto_increment_key<K: Default + 'static>(
    last_id: impl Into<i128>,
) -> crate::Result<K> {
    let last_id = last_id.into();
    let key_type = std::any::TypeId::of::<K>();
    if key_type == std::any::TypeId::of::<()>() {
        Ok(downcast_auto_increment_key(()))
    } else if key_type == std::any::TypeId::of::<i32>() {
        Ok(downcast_auto_increment_key(last_id as i32))
    } else if key_type == std::any::TypeId::of::<i64>() {
        Ok(downcast_auto_increment_key(last_id as i64))
    } else if key_type == std::any::TypeId::of::<u32>() {
        Ok(downcast_auto_increment_key(last_id as u32))
    } else if key_type == std::any::TypeId::of::<u64>() {
        Ok(downcast_auto_increment_key(last_id as u64))
    } else if key_type == std::any::TypeId::of::<usize>() {
        Ok(downcast_auto_increment_key(last_id as usize))
    } else if key_type == std::any::TypeId::of::<Option<i64>>() {
        Ok(downcast_auto_increment_key(Some(last_id as i64)))
    } else {
        Err(crate::ormer_error!(
            "Unsupported auto-increment key type. Only i32, i64, u32, u64, usize, Option<i64> and () are supported."
        ))
    }
}

fn build_batch_insert_sql_with_prefix(
    db_type: DbType,
    insert_prefix: &str,
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    let table_name = quote_qualified_identifier(db_type, table_name);
    let columns_str = quote_column_list(db_type, columns);
    let col_count = columns.len();

    let mut sql = format!("{insert_prefix} {table_name} ({columns_str}) VALUES ");

    for idx in 0..models_count {
        if idx > 0 {
            sql.push_str(", ");
        }

        let start_idx = idx * col_count + 1;
        sql.push_str(&format!(
            "({})",
            placeholder_list(db_type, start_idx, col_count)
        ));
    }

    (sql, col_count)
}

/// 构建批量插入 SQL 的公共函数（使用自定义列名列表，排除自增主键）
pub fn build_batch_insert_sql_with_columns(
    db_type: DbType,
    table_name: &str,
    columns: &[&str],
    models_count: usize,
) -> (String, usize) {
    build_batch_insert_sql_with_prefix(db_type, "INSERT INTO", table_name, columns, models_count)
}

#[derive(Debug, Clone, Copy)]
pub enum BatchInsertValuesMode {
    All,
    WithoutAutoIncrement,
}

/// 构建批量 INSERT 主体并收集对应参数，冲突子句由各后端追加。
pub fn build_batch_insert_statement<T: Model>(
    db_type: DbType,
    insert_prefix: &str,
    table_name: &str,
    columns: &[&str],
    models: &[&T],
    values_mode: BatchInsertValuesMode,
) -> (String, Vec<Value>) {
    let (sql, _) = build_batch_insert_sql_with_prefix(
        db_type,
        insert_prefix,
        table_name,
        columns,
        models.len(),
    );
    let values = match values_mode {
        BatchInsertValuesMode::All => collect_batch_insert_values(models),
        BatchInsertValuesMode::WithoutAutoIncrement => {
            collect_batch_insert_values_with_auto_increment(models)
        }
    };
    (sql, values)
}

pub fn auto_increment_column<T: Model>() -> Option<&'static str> {
    T::column_schema()
        .iter()
        .find(|column| column.is_auto_increment)
        .map(|column| column.name)
}

pub fn append_auto_increment_returning<T: Model>(db_type: DbType, sql: String) -> String {
    let Some(_pk_col) = auto_increment_column::<T>() else {
        return sql;
    };

    match db_type {
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => format!("{sql} RETURNING rowid"),
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            let pk_col = quote_column_list(DbType::PostgreSQL, &[_pk_col]);
            format!("{sql} RETURNING {pk_col}")
        }
        #[cfg(feature = "mysql")]
        DbType::MySQL => sql,
        #[cfg(feature = "mssql")]
        DbType::MSSQL => {
            let output = format!(
                " OUTPUT {}",
                quote_column_with_prefix(DbType::MSSQL, "inserted", _pk_col)
            );
            if let Some(pos) = sql.rfind(" DEFAULT VALUES") {
                let mut sql = sql;
                sql.insert_str(pos, &output);
                sql
            } else if let Some(pos) = sql.rfind(" VALUES ") {
                let mut sql = sql;
                sql.insert_str(pos, &output);
                sql
            } else {
                format!("{sql}{output}")
            }
        }
    }
}

pub fn build_insert_statement<T: Model>(db_type: DbType, models: &[&T]) -> (String, Vec<Value>) {
    let columns = T::insert_columns();
    build_batch_insert_statement::<T>(
        db_type,
        "INSERT INTO",
        T::table_name_for_db(db_type),
        &columns,
        models,
        BatchInsertValuesMode::WithoutAutoIncrement,
    )
}

fn routed_table_name_for_models<T: Model>(db_type: DbType, models: &[&T]) -> crate::Result<String> {
    let Some(first) = models.first() else {
        return Ok(T::table_name_for_db(db_type).to_string());
    };
    let first_route = first.table_route()?;
    let table_name = routed_table_name_for_db(db_type, T::TABLE_NAME, &first_route)?;

    for model in models.iter().skip(1) {
        let route = model.table_route()?;
        let model_table = routed_table_name_for_db(db_type, T::TABLE_NAME, &route)?;
        if model_table != table_name {
            return Err(crate::ormer_error!(
                "Batch insert cannot target multiple routed tables: {} and {}",
                table_name,
                model_table
            ));
        }
    }

    Ok(table_name)
}

pub fn routed_insert_table_name<T: Model>(db_type: DbType, models: &[&T]) -> crate::Result<String> {
    routed_table_name_for_models(db_type, models)
}

pub fn build_routed_insert_statement<T: Model>(
    db_type: DbType,
    models: &[&T],
) -> crate::Result<(String, Vec<Value>)> {
    let columns = T::insert_columns();
    let table_name = routed_table_name_for_models(db_type, models)?;
    Ok(build_batch_insert_statement::<T>(
        db_type,
        "INSERT INTO",
        &table_name,
        &columns,
        models,
        BatchInsertValuesMode::WithoutAutoIncrement,
    ))
}

#[derive(Debug, Clone)]
pub struct PartialInsertStatement {
    pub sql: String,
    pub params: Vec<Value>,
    pub param_rust_types: Vec<&'static str>,
}

pub fn build_partial_insert_statement<T: Model>(
    db_type: DbType,
    assignments: &[InsertAssignment],
) -> crate::Result<PartialInsertStatement> {
    build_partial_insert_statement_for_table::<T>(
        db_type,
        assignments,
        T::table_name_for_db(db_type),
    )
}

pub fn build_partial_insert_statement_for_table<T: Model>(
    db_type: DbType,
    assignments: &[InsertAssignment],
    table_name: &str,
) -> crate::Result<PartialInsertStatement> {
    let mut values_by_column = HashMap::<&'static str, InsertValue>::new();
    for assignment in assignments {
        let column_name = T::column_name_for_field(&assignment.column).ok_or_else(|| {
            crate::ormer_error!(
                "Column {} not found on model {}",
                assignment.column,
                T::TABLE_NAME
            )
        })?;
        values_by_column.insert(column_name, assignment.value.clone());
    }

    let mut columns = Vec::new();
    let mut params = Vec::new();
    let mut param_rust_types = Vec::new();
    for schema in T::column_schema() {
        match values_by_column.get(schema.name) {
            Some(InsertValue::Literal(value)) => {
                columns.push(schema.name);
                params.push(value.clone());
                param_rust_types.push(schema.data_type.unwrap_or(schema.rust_type));
            }
            Some(InsertValue::Default) | None => {}
        }
    }

    let table_name = quote_qualified_identifier(db_type, table_name);
    let sql = if columns.is_empty() {
        match db_type {
            #[cfg(feature = "mysql")]
            DbType::MySQL => format!("INSERT INTO {table_name} () VALUES ()"),
            _ => format!("INSERT INTO {table_name} DEFAULT VALUES"),
        }
    } else {
        let columns_str = quote_column_list(db_type, &columns);
        let placeholders = placeholder_list(db_type, 1, columns.len());
        format!("INSERT INTO {table_name} ({columns_str}) VALUES ({placeholders})")
    };

    Ok(PartialInsertStatement {
        sql,
        params,
        param_rust_types,
    })
}

pub fn build_partial_insert_statement_with_auto_increment_returning<T: Model>(
    db_type: DbType,
    assignments: &[InsertAssignment],
) -> crate::Result<PartialInsertStatement> {
    let mut statement = build_partial_insert_statement::<T>(db_type, assignments)?;
    statement.sql = append_auto_increment_returning::<T>(db_type, statement.sql);
    Ok(statement)
}

pub fn build_partial_insert_statement_with_auto_increment_returning_for_table<T: Model>(
    db_type: DbType,
    assignments: &[InsertAssignment],
    table_name: &str,
) -> crate::Result<PartialInsertStatement> {
    let mut statement =
        build_partial_insert_statement_for_table::<T>(db_type, assignments, table_name)?;
    statement.sql = append_auto_increment_returning::<T>(db_type, statement.sql);
    Ok(statement)
}

pub fn validate_insert_model_table<T: Model>(
    db_type: DbType,
    source_table: Option<&'static str>,
) -> crate::Result<()> {
    let Some(source_table) = source_table else {
        return Ok(());
    };
    let target_table = T::TABLE_NAME;
    let db_table = T::table_name_for_db(db_type);
    if source_table == target_table || source_table == db_table {
        return Ok(());
    }

    Err(crate::ormer_error!(
        "Insert model targets table {}, but model {} uses table {}",
        source_table,
        std::any::type_name::<T>(),
        target_table
    ))
}

fn insert_prefix_for_conflict(db_type: DbType, _conflict: Option<&InsertConflict>) -> &'static str {
    match db_type {
        #[cfg(feature = "mysql")]
        DbType::MySQL
            if _conflict.and_then(|conflict| conflict.action)
                == Some(InsertConflictAction::DoNothing) =>
        {
            "INSERT IGNORE INTO"
        }
        _ => "INSERT INTO",
    }
}

pub fn build_insert_statement_with_conflict<T: Model>(
    db_type: DbType,
    models: &[&T],
    conflict: Option<&InsertConflict>,
) -> crate::Result<(String, Vec<Value>)> {
    let columns = T::insert_columns();
    let table_name = routed_table_name_for_models(db_type, models)?;
    let (mut sql, mut values) = build_batch_insert_statement::<T>(
        db_type,
        insert_prefix_for_conflict(db_type, conflict),
        &table_name,
        &columns,
        models,
        BatchInsertValuesMode::WithoutAutoIncrement,
    );

    if let Some(conflict) = conflict
        && conflict.is_configured()
    {
        append_insert_conflict_clause::<T>(db_type, &mut sql, &mut values, conflict)?;
    }

    Ok((sql, values))
}

fn insert_rows_per_statement<T: Model>(db_type: DbType) -> usize {
    let column_count = T::insert_columns().len().max(1);
    (bind_param_limit(db_type) / column_count).max(1)
}

pub fn build_insert_statements_with_conflict<T: Model>(
    db_type: DbType,
    models: &[&T],
    conflict: Option<&InsertConflict>,
) -> crate::Result<Vec<InsertSqlStatement>> {
    if models.is_empty() {
        return Ok(Vec::new());
    }

    if conflict.is_some_and(InsertConflict::is_configured) || auto_increment_column::<T>().is_some()
    {
        let (sql, params) = build_insert_statement_with_conflict::<T>(db_type, models, conflict)?;
        return Ok(vec![InsertSqlStatement {
            sql,
            params,
            row_count: models.len(),
        }]);
    }

    let rows_per_statement = insert_rows_per_statement::<T>(db_type);
    if models.len() <= rows_per_statement {
        let (sql, params) = build_insert_statement_with_conflict::<T>(db_type, models, conflict)?;
        return Ok(vec![InsertSqlStatement {
            sql,
            params,
            row_count: models.len(),
        }]);
    }

    routed_table_name_for_models(db_type, models)?;
    models
        .chunks(rows_per_statement)
        .map(|chunk| {
            let (sql, params) =
                build_insert_statement_with_conflict::<T>(db_type, chunk, conflict)?;
            Ok(InsertSqlStatement {
                sql,
                params,
                row_count: chunk.len(),
            })
        })
        .collect()
}

fn append_insert_conflict_clause<T: Model>(
    #[allow(unused_variables)] db_type: DbType,
    #[allow(unused_variables)] sql: &mut String,
    #[allow(unused_variables)] params: &mut Vec<Value>,
    #[allow(unused_variables)] conflict: &InsertConflict,
) -> crate::Result<()> {
    match db_type {
        #[cfg(feature = "postgresql")]
        DbType::PostgreSQL => {
            append_standard_insert_conflict_clause::<T>(DbType::PostgreSQL, sql, params, conflict)
        }
        #[cfg(feature = "sqlite")]
        DbType::Sqlite => {
            append_standard_insert_conflict_clause::<T>(DbType::Sqlite, sql, params, conflict)
        }
        #[cfg(feature = "mysql")]
        DbType::MySQL => append_mysql_insert_conflict_clause(sql, params, conflict),
        #[cfg(feature = "mssql")]
        DbType::MSSQL => Err(crate::ormer_error!(
            "MSSQL does not support configurable insert conflict handling; use insert_or_update for primary-key MERGE"
        )),
    }
}

#[allow(dead_code)]
fn append_standard_insert_conflict_clause<T: Model>(
    db_type: DbType,
    sql: &mut String,
    params: &mut Vec<Value>,
    conflict: &InsertConflict,
) -> crate::Result<()> {
    let action = conflict.action.ok_or_else(|| {
        crate::ormer_error!("insert conflict handling requires do_nothing, do_update, or set")
    })?;

    sql.push_str(" ON CONFLICT");
    append_standard_conflict_target(db_type, sql, params, conflict)?;

    match action {
        InsertConflictAction::DoNothing => {
            if conflict.update_filter.is_some() || !conflict.assignments.is_empty() {
                return Err(crate::ormer_error!(
                    "do_nothing cannot be combined with update filter or set assignments"
                ));
            }
            sql.push_str(" DO NOTHING");
        }
        InsertConflictAction::DoUpdate => {
            if conflict.target.is_none() {
                return Err(crate::ormer_error!(
                    "do_update conflict handling requires on_conflict or on_constraint"
                ));
            }
            if conflict.assignments.is_empty() {
                return Err(crate::ormer_error!(
                    "do_update conflict handling requires at least one set assignment"
                ));
            }

            sql.push_str(" DO UPDATE SET ");
            for (index, assignment) in conflict.assignments.iter().enumerate() {
                if index > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&format_upsert_update_assignment(
                    db_type, assignment, params,
                ));
            }
            if let Some(filter) = &conflict.update_filter {
                sql.push_str(" WHERE ");
                let mut param_idx = params.len() + 1;
                format_filter_with_params(filter, sql, &mut param_idx, params, db_type)?;
            }
        }
    }

    Ok(())
}

#[allow(dead_code, unreachable_patterns)]
fn append_standard_conflict_target(
    db_type: DbType,
    sql: &mut String,
    params: &mut Vec<Value>,
    conflict: &InsertConflict,
) -> crate::Result<()> {
    match &conflict.target {
        Some(InsertConflictTarget::Columns(columns)) => {
            if columns.is_empty() {
                return Err(crate::ormer_error!(
                    "on_conflict requires at least one conflict target column"
                ));
            }
            sql.push_str(" (");
            sql.push_str(&quote_column_list(db_type, columns));
            sql.push(')');
            if let Some(filter) = &conflict.target_filter {
                sql.push_str(" WHERE ");
                let mut param_idx = params.len() + 1;
                format_filter_with_params(filter, sql, &mut param_idx, params, db_type)?;
            }
        }
        Some(InsertConflictTarget::Constraint(_name)) => match db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => {
                if conflict.target_filter.is_some() {
                    return Err(crate::ormer_error!(
                        "conflict_where cannot be combined with on_constraint"
                    ));
                }
                sql.push_str(" ON CONSTRAINT ");
                sql.push_str(&quote_identifier(db_type, _name));
            }
            _ => {
                return Err(crate::ormer_error!(
                    "on_constraint is only supported for PostgreSQL insert conflict handling"
                ));
            }
        },
        None => {
            if conflict.target_filter.is_some() {
                return Err(crate::ormer_error!(
                    "conflict_where requires on_conflict column targets"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(feature = "mysql")]
fn append_mysql_insert_conflict_clause(
    sql: &mut String,
    params: &mut Vec<Value>,
    conflict: &InsertConflict,
) -> crate::Result<()> {
    if conflict.target.is_some() {
        return Err(crate::ormer_error!(
            "MySQL ON DUPLICATE KEY cannot target a specific unique key or constraint"
        ));
    }
    if conflict.target_filter.is_some() {
        return Err(crate::ormer_error!(
            "MySQL ON DUPLICATE KEY cannot target a partial unique index"
        ));
    }

    let action = conflict.action.ok_or_else(|| {
        crate::ormer_error!("insert conflict handling requires do_nothing, do_update, or set")
    })?;

    match action {
        InsertConflictAction::DoNothing => {
            if conflict.update_filter.is_some() || !conflict.assignments.is_empty() {
                return Err(crate::ormer_error!(
                    "do_nothing cannot be combined with update filter or set assignments"
                ));
            }
        }
        InsertConflictAction::DoUpdate => {
            if conflict.update_filter.is_some() {
                return Err(crate::ormer_error!(
                    "MySQL ON DUPLICATE KEY UPDATE does not support DO UPDATE WHERE"
                ));
            }
            if conflict.assignments.is_empty() {
                return Err(crate::ormer_error!(
                    "do_update conflict handling requires at least one set assignment"
                ));
            }
            sql.push_str(" ON DUPLICATE KEY UPDATE ");
            for (index, assignment) in conflict.assignments.iter().enumerate() {
                if index > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&format_upsert_update_assignment(
                    DbType::MySQL,
                    assignment,
                    params,
                ));
            }
        }
    }

    Ok(())
}

pub fn build_insert_statement_with_auto_increment_returning<T: Model>(
    db_type: DbType,
    models: &[&T],
) -> crate::Result<(String, Vec<Value>)> {
    let (sql, values) = build_routed_insert_statement::<T>(db_type, models)?;
    Ok((append_auto_increment_returning::<T>(db_type, sql), values))
}

pub fn build_insert_statement_with_conflict_and_auto_increment_returning<T: Model>(
    db_type: DbType,
    models: &[&T],
    conflict: Option<&InsertConflict>,
) -> crate::Result<(String, Vec<Value>)> {
    let (sql, values) = build_insert_statement_with_conflict::<T>(db_type, models, conflict)?;
    Ok((append_auto_increment_returning::<T>(db_type, sql), values))
}

#[cfg(feature = "mssql")]
pub fn build_mssql_merge_source<T: Model>(models: &[&T]) -> (String, Vec<Value>) {
    let columns = quote_column_list(DbType::MSSQL, T::COLUMNS);
    let col_count = T::COLUMNS.len();
    let mut sql = format!(
        "MERGE INTO {} AS target USING (VALUES ",
        quote_table_name::<T>(DbType::MSSQL)
    );
    let mut all_values = Vec::new();

    for (idx, model) in models.iter().enumerate() {
        if idx > 0 {
            sql.push_str(", ");
        }
        let placeholders = placeholder_list(DbType::MSSQL, all_values.len() + 1, col_count);
        sql.push_str(&format!("({placeholders})"));
        all_values.extend(model.field_values());
    }

    sql.push_str(&format!(") AS source ({columns}) ON "));
    for (i, pk) in T::primary_key_columns().iter().enumerate() {
        if i > 0 {
            sql.push_str(" AND ");
        }
        sql.push_str(&format!(
            "{} = {}",
            quote_column_with_prefix(DbType::MSSQL, "target", pk),
            quote_column_with_prefix(DbType::MSSQL, "source", pk)
        ));
    }

    (sql, all_values)
}

#[cfg(feature = "mssql")]
pub fn append_mssql_merge_update_clause<T: Model>(sql: &mut String) {
    sql.push_str(" WHEN MATCHED THEN UPDATE SET ");
    let pks = T::primary_key_columns();
    let mut first = true;
    for col_name in T::COLUMNS.iter() {
        if pks.contains(col_name) {
            continue;
        }
        if !first {
            sql.push_str(", ");
        }
        sql.push_str(&quote_assignment(
            DbType::MSSQL,
            col_name,
            &quote_column_with_prefix(DbType::MSSQL, "source", col_name),
        ));
        first = false;
    }
}

#[cfg(feature = "mssql")]
pub fn append_mssql_merge_insert_clause<T: Model>(sql: &mut String) {
    let columns = quote_column_list(DbType::MSSQL, T::COLUMNS);
    sql.push_str(&format!(
        " WHEN NOT MATCHED THEN INSERT ({columns}) VALUES ("
    ));
    for (i, col_name) in T::COLUMNS.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&quote_column_with_prefix(DbType::MSSQL, "source", col_name));
    }
    sql.push_str(");");
}

/// 收集批量插入的所有模型值
pub fn collect_batch_insert_values<T: Model>(models: &[&T]) -> Vec<Value> {
    let mut all_values = Vec::new();
    for model in models {
        let values = model.field_values();
        all_values.extend(values);
    }
    all_values
}

/// 收集批量插入的所有模型值（排除自增主键）
pub fn collect_batch_insert_values_with_auto_increment<T: Model>(models: &[&T]) -> Vec<Value> {
    let mut all_values = Vec::new();
    for model in models {
        let values = model.insert_values();
        all_values.extend(values);
    }
    all_values
}

/// 统一的列值解析函数 - 严格模式
///
/// 用于流式查询中解析列值,非空字段解析失败时返回错误而非默认值
#[allow(clippy::too_many_arguments)]
pub fn parse_column_value_strict(
    rust_type: &str,
    is_nullable: bool,
    column_name: &str,
    get_int: impl FnOnce() -> Option<i64>,
    get_string: impl FnOnce() -> Option<String>,
    get_real: impl FnOnce() -> Option<f64>,
    get_bool: impl FnOnce() -> Option<i8>,
    get_bytes: impl FnOnce() -> Option<Vec<u8>>,
    get_datetime: impl FnOnce() -> Option<chrono::DateTime<chrono::Utc>>,
) -> crate::Result<Value> {
    parse_column_value_options(
        rust_type,
        is_nullable,
        get_int(),
        get_string(),
        get_real(),
        get_bool(),
        get_bytes(),
        get_datetime(),
        ColumnValueMode::Strict { column_name },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_insert_builder_uses_backend_placeholder_rules() {
        let columns = &["id", "name", "age"];

        #[cfg(feature = "sqlite")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::Sqlite, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES (?, ?, ?), (?, ?, ?)"
            );
        }

        #[cfg(feature = "mysql")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::MySQL, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES (?, ?, ?), (?, ?, ?)"
            );
        }

        #[cfg(feature = "postgresql")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::PostgreSQL, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES ($1, $2, $3), ($4, $5, $6)"
            );
        }

        #[cfg(feature = "mssql")]
        {
            let (sql, col_count) =
                build_batch_insert_sql_with_columns(DbType::MSSQL, "users", columns, 2);

            assert_eq!(col_count, 3);
            assert_eq!(
                sql,
                "INSERT INTO users (id, name, age) VALUES (@P1, @P2, @P3), (@P4, @P5, @P6)"
            );
        }
    }
}
