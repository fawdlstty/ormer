use crate::abstract_layer::DbType;
use crate::abstract_layer::common::common_helpers::placeholder;
use crate::model::{Value, normalize_table_name_for_db, quote_column_reference};
use crate::query::expr::SqlExpr;
use crate::query::filter::FilterExpr;

/// SQLite 上 Decimal 家族以 TEXT 存储，数值语义的比较需要显式
/// `CAST(... AS NUMERIC)`；comparison_sql / Between / IN 共用此判定。
fn sqlite_needs_numeric_cast(db_type: DbType, value: &Value) -> bool {
    #[cfg(feature = "sqlite")]
    {
        matches!(db_type, DbType::Sqlite)
            && matches!(value, Value::Decimal(_) | Value::BigDecimal(_))
    }
    #[cfg(not(feature = "sqlite"))]
    {
        let _ = (db_type, value);
        false
    }
}

/// 通用的 WHERE 条件格式化器
///
/// 用于将 FilterExpr 格式化为 SQL WHERE 子句，并收集参数
pub struct FilterFormatter {
    db_type: DbType,
    /// 表别名前缀，例如 "t0" 用于多表查询
    table_prefix: Option<String>,
    /// 右列表别名前缀，用于 ColumnComparison（列-列比较）
    right_table_prefix: Option<String>,
    /// 关联表列清单（别名 → SQL 列名），用于值过滤列的表归属解析
    related_tables: Vec<(&'static str, Vec<&'static str>)>,
    /// PostgreSQL HAVING子句中的参数需要添加::bigint类型转换
    postgresql_having_cast: bool,
}

impl FilterFormatter {
    pub fn new(db_type: DbType) -> Self {
        Self {
            db_type,
            table_prefix: None,
            right_table_prefix: None,
            related_tables: Vec::new(),
            postgresql_having_cast: false,
        }
    }

    /// 设置表别名前缀
    pub fn with_table_prefix(mut self, prefix: &str) -> Self {
        self.table_prefix = Some(prefix.to_string());
        self
    }

    /// 设置右列表别名前缀（用于列-列比较）
    pub fn with_right_table_prefix(mut self, prefix: &str) -> Self {
        self.right_table_prefix = Some(prefix.to_string());
        self
    }

    /// 注册关联表（别名 → SQL 列名清单）。
    ///
    /// 多表查询中，通过 Where 代理生成的值过滤列（Comparison / IN / BETWEEN /
    /// IS NULL 等）无法在表达式层面区分来自哪张表；渲染时按注册顺序解析：
    /// 未限定的过滤列命中某个关联表的列清单时，限定到该表别名（t1/t2/t3），
    /// 否则回落到主表前缀 t0。列-列比较（ColumnComparison）仍按
    /// `table_prefix` / `right_table_prefix` 的位置约定渲染，不受影响。
    pub fn with_related_tables(
        mut self,
        tables: Vec<(&'static str, Vec<&'static str>)>,
    ) -> Self {
        self.related_tables = tables;
        self
    }

    /// 设置PostgreSQL HAVING子句类型转换标志
    pub fn with_postgresql_having_cast(mut self, cast: bool) -> Self {
        self.postgresql_having_cast = cast;
        self
    }

    /// 格式化为 SQL WHERE 子句并收集参数
    ///
    /// # 参数
    /// * `filter` - 过滤表达式
    /// * `param_idx` - 参数索引（用于 PostgreSQL 的 $1, $2 等）
    /// * `params` - 输出参数列表
    ///
    /// # 返回
    /// 格式化后的 SQL WHERE 子句（不含 WHERE 关键字）
    pub fn format(
        &self,
        filter: &FilterExpr,
        param_idx: &mut i32,
        params: &mut Vec<Value>,
    ) -> String {
        let mut sql = String::new();
        self.format_recursive(filter, &mut sql, param_idx, params);
        sql
    }

    pub(crate) fn full_text_search_sql(
        &self,
        search: &crate::query::filter::FullTextQuery,
        param_idx: &mut i32,
        params: &mut Vec<Value>,
    ) -> String {
        // 空查询/空字段列表在校验路径（validate_filter_for_db）已报错；
        // 这里是未走校验的渲染路径的兜底，渲染错误占位（零占位符）。
        // 判定必须先于任何参数 push（含下方 query_sql），保证参数计数
        // 与占位符数量对齐。
        if search.query.trim().is_empty() {
            return invalid_filter_marker(
                "full-text search requires a non-empty query (call .query(\"...\"))",
            );
        }
        if search.exprs.is_empty() {
            return invalid_filter_marker("full-text search requires at least one field");
        }
        let expr_sql = |expr: &SqlExpr, param_idx: &mut i32, params: &mut Vec<Value>| {
            expr.to_sql(
                self.db_type,
                param_idx,
                params,
                self.table_prefix.as_deref(),
            )
        };
        let query_expr = SqlExpr::Value(Value::Text(search.query.clone()));
        let query_sql = query_expr.to_sql(
            self.db_type,
            param_idx,
            params,
            self.table_prefix.as_deref(),
        );

        match self.db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => {
                // language 未显式设置时内联 'simple' 字面量而不产生绑定参数，
                // 与 collect_filter_param_rust_types 的收集条件（language.is_some
                // 才收集 String）保持一致；显式设置时仍渲染为占位符。
                let language_sql = match search.language.as_deref() {
                    Some(language) => SqlExpr::Value(Value::Text(language.to_string())).to_sql(
                        self.db_type,
                        param_idx,
                        params,
                        None,
                    ),
                    None => "'simple'".to_string(),
                };
                let fields = search
                    .exprs
                    .iter()
                    .map(|expr| expr_sql(expr, param_idx, params))
                    .collect::<Vec<_>>()
                    .join(", ");
                let vector = format!("to_tsvector({}, COALESCE({fields}, ''))", language_sql);
                let query_fn = match search.mode {
                    crate::query::filter::FullTextMode::Natural => "plainto_tsquery",
                    crate::query::filter::FullTextMode::Boolean => "to_tsquery",
                    crate::query::filter::FullTextMode::WebSearch => "websearch_to_tsquery",
                };
                format!("({vector}) @@ {query_fn}({language_sql}, {query_sql})")
            }
            #[cfg(feature = "mysql")]
            DbType::MySQL => {
                let fields = search
                    .exprs
                    .iter()
                    .map(|expr| expr_sql(expr, param_idx, params))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mode = match search.mode {
                    crate::query::filter::FullTextMode::Natural => " IN NATURAL LANGUAGE MODE",
                    crate::query::filter::FullTextMode::Boolean => " IN BOOLEAN MODE",
                    crate::query::filter::FullTextMode::WebSearch => " IN NATURAL LANGUAGE MODE",
                };
                format!("MATCH ({fields}) AGAINST ({query_sql}{mode})")
            }
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => {
                if search.exprs.len() == 1 {
                    format!(
                        "{} MATCH {query_sql}",
                        expr_sql(&search.exprs[0], param_idx, params)
                    )
                } else {
                    let clauses = search
                        .exprs
                        .iter()
                        .map(|expr| {
                            format!("{} LIKE {query_sql}", expr_sql(expr, param_idx, params))
                        })
                        .collect::<Vec<_>>()
                        .join(" OR ");
                    format!("({clauses})")
                }
            }
            #[cfg(feature = "mssql")]
            DbType::MSSQL => {
                let fields = search
                    .exprs
                    .iter()
                    .map(|expr| expr_sql(expr, param_idx, params))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("CONTAINS (({fields}), {query_sql})")
            }
            #[cfg(feature = "duckdb")]
            DbType::DuckDB => {
                let clauses = search
                    .exprs
                    .iter()
                    .map(|expr| {
                        format!(
                            "lower({}) LIKE lower({query_sql})",
                            expr_sql(expr, param_idx, params)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" OR ");
                format!("({clauses})")
            }
            #[cfg(feature = "clickhouse")]
            DbType::ClickHouse => {
                let clauses = search
                    .exprs
                    .iter()
                    .map(|expr| {
                        format!(
                            "multiSearchAnyCaseInsensitive({}, [{query_sql}])",
                            expr_sql(expr, param_idx, params)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" OR ");
                format!("({clauses})")
            }
            #[cfg(feature = "questdb")]
            DbType::QuestDB => {
                invalid_filter_marker("full-text search is not supported on QuestDB")
            }
            #[cfg(feature = "influxdb")]
            _ => invalid_filter_marker("full-text search is not supported on InfluxDB"),
        }
    }

    fn format_recursive(
        &self,
        expr: &FilterExpr,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<Value>,
    ) {
        match expr {
            FilterExpr::Comparison {
                column,
                operator,
                value,
            } => {
                let full_col_name = self.qualified_column(column);
                use std::fmt::Write;
                // 三值逻辑下 "= NULL" 恒假、"/!= NULL" 恒真，
                // 改写为 IS NULL / IS NOT NULL，且不产生绑定参数
                if matches!(value, Value::Null) && matches!(operator.as_str(), "=" | "!=" | "<>") {
                    let null_check = if operator == "=" {
                        "IS NULL"
                    } else {
                        "IS NOT NULL"
                    };
                    write!(
                        sql,
                        "{} {}",
                        quote_column_reference(self.db_type, &full_col_name),
                        null_check
                    )
                    .unwrap_or_else(|e| panic!("Failed to write SQL WHERE clause: {}", e));
                    return;
                }
                // NULL + 非等值操作符（> >= < <= LIKE 等）在三值逻辑下恒为
                // UNKNOWN，行级过滤中等价于恒假。校验阶段
                // （FilterExpr::validate_null_usage）已对主查询路径报错；
                // 这里是未走校验的渲染路径的兜底，渲染显式恒假表达式并
                // 不产生绑定参数，避免静默生成 "col" > NULL 这类无警告假谓词
                if matches!(value, Value::Null) {
                    write!(sql, "1 = 0")
                        .unwrap_or_else(|e| panic!("Failed to write SQL WHERE clause: {}", e));
                    return;
                }
                write!(
                    sql,
                    "{}",
                    self.comparison_sql(
                        &quote_column_reference(self.db_type, &full_col_name),
                        operator,
                        param_idx,
                        value,
                    )
                )
                .unwrap_or_else(|e| panic!("Failed to write SQL WHERE clause: {}", e));

                params.push(value.clone().into());
                *param_idx += 1;
            }
            FilterExpr::ColumnComparison {
                left_column,
                operator,
                right_column,
            } => {
                let left_col = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, left_column)
                } else {
                    left_column.clone()
                };

                let right_col = if let Some(ref prefix) = self.right_table_prefix {
                    format!("{}.{}", prefix, right_column)
                } else {
                    right_column.clone()
                };

                use std::fmt::Write;
                write!(
                    sql,
                    "{} {} {}",
                    quote_column_reference(self.db_type, &left_col),
                    operator,
                    quote_column_reference(self.db_type, &right_col)
                )
                .unwrap_or_else(|e| panic!("Failed to write column comparison SQL: {}", e));
            }
            FilterExpr::In { column, values } => {
                self.format_column_values_clause(sql, column, "IN", values, param_idx, params);
            }
            FilterExpr::NotIn { column, values } => {
                self.format_column_values_clause(sql, column, "NOT IN", values, param_idx, params);
            }
            FilterExpr::InSubquery {
                column,
                subquery_sql,
                subquery_params,
            } => {
                use std::fmt::Write;
                let subquery_sql =
                    rebase_subquery_sql(subquery_sql, self.db_type, *param_idx as usize - 1);
                write!(sql, "{} IN ({})", self.quoted_column(column), subquery_sql)
                    .unwrap_or_else(|e| panic!("Failed to write subquery IN clause: {}", e));
                self.append_subquery_params(subquery_params, param_idx, params);
            }
            FilterExpr::InSubqueryDynamic { column, subquery } => {
                use std::fmt::Write;
                let (subquery_sql, subquery_params) =
                    render_dynamic_subquery(subquery, self.db_type);
                let subquery_sql =
                    rebase_subquery_sql(&subquery_sql, self.db_type, *param_idx as usize - 1);
                write!(sql, "{} IN ({})", self.quoted_column(column), subquery_sql)
                    .unwrap_or_else(|e| panic!("Failed to write dynamic subquery IN clause: {}", e));
                self.append_subquery_params(&subquery_params, param_idx, params);
            }
            FilterExpr::NotInSubquery {
                column,
                subquery_sql,
                subquery_params,
            } => {
                use std::fmt::Write;
                let subquery_sql =
                    rebase_subquery_sql(subquery_sql, self.db_type, *param_idx as usize - 1);
                write!(
                    sql,
                    "{} NOT IN ({})",
                    self.quoted_column(column),
                    subquery_sql
                )
                .unwrap_or_else(|e| panic!("Failed to write subquery NOT IN clause: {}", e));
                self.append_subquery_params(subquery_params, param_idx, params);
            }
            FilterExpr::NotInSubqueryDynamic { column, subquery } => {
                use std::fmt::Write;
                let (subquery_sql, subquery_params) =
                    render_dynamic_subquery(subquery, self.db_type);
                let subquery_sql =
                    rebase_subquery_sql(&subquery_sql, self.db_type, *param_idx as usize - 1);
                write!(
                    sql,
                    "{} NOT IN ({})",
                    self.quoted_column(column),
                    subquery_sql
                )
                .unwrap_or_else(|e| {
                    panic!("Failed to write dynamic subquery NOT IN clause: {}", e)
                });
                self.append_subquery_params(&subquery_params, param_idx, params);
            }
            FilterExpr::And(left, right) => {
                sql.push('(');
                self.format_recursive(left, sql, param_idx, params);
                sql.push_str(" AND ");
                self.format_recursive(right, sql, param_idx, params);
                sql.push(')');
            }
            FilterExpr::Or(left, right) => {
                sql.push('(');
                self.format_recursive(left, sql, param_idx, params);
                sql.push_str(" OR ");
                self.format_recursive(right, sql, param_idx, params);
                sql.push(')');
            }
            FilterExpr::IsNull { column } => {
                use std::fmt::Write;
                write!(sql, "{} IS NULL", self.quoted_column(column))
                    .unwrap_or_else(|e| panic!("Failed to write IS NULL clause: {}", e));
            }
            FilterExpr::IsNotNull { column } => {
                use std::fmt::Write;
                write!(sql, "{} IS NOT NULL", self.quoted_column(column))
                    .unwrap_or_else(|e| panic!("Failed to write IS NOT NULL clause: {}", e));
            }
            FilterExpr::Between { column, min, max } => {
                let col_name = self.qualified_column(column);
                use std::fmt::Write;
                let min_placeholder = placeholder(self.db_type, *param_idx as usize);
                *param_idx += 1;
                let max_placeholder = placeholder(self.db_type, *param_idx as usize);
                *param_idx += 1;
                // 与 comparison_sql 对齐：SQLite 上 Decimal 走 BETWEEN 时
                // 两侧同样 CAST AS NUMERIC，避免 TEXT 比较漏行。
                let numeric_cast = sqlite_needs_numeric_cast(self.db_type, min)
                    || sqlite_needs_numeric_cast(self.db_type, max);
                let col_sql = quote_column_reference(self.db_type, &col_name);
                let (col_sql, min_placeholder, max_placeholder) = if numeric_cast {
                    (
                        format!("CAST({col_sql} AS NUMERIC)"),
                        format!("CAST({min_placeholder} AS NUMERIC)"),
                        format!("CAST({max_placeholder} AS NUMERIC)"),
                    )
                } else {
                    (col_sql, min_placeholder, max_placeholder)
                };
                write!(
                    sql,
                    "{} BETWEEN {} AND {}",
                    col_sql, min_placeholder, max_placeholder
                )
                .unwrap_or_else(|e| panic!("Failed to write BETWEEN clause: {}", e));
                params.push(min.clone().into());
                params.push(max.clone().into());
            }
            FilterExpr::Exists {
                subquery_sql,
                subquery_params,
            } => {
                use std::fmt::Write;
                let subquery_sql =
                    rebase_subquery_sql(subquery_sql, self.db_type, *param_idx as usize - 1);
                write!(sql, "EXISTS ({})", subquery_sql)
                    .unwrap_or_else(|e| panic!("Failed to write EXISTS clause: {}", e));
                self.append_subquery_params(subquery_params, param_idx, params);
            }
            FilterExpr::ExistsDynamic { subquery } => {
                use std::fmt::Write;
                let (subquery_sql, subquery_params) =
                    render_dynamic_subquery(subquery, self.db_type);
                let subquery_sql =
                    rebase_subquery_sql(&subquery_sql, self.db_type, *param_idx as usize - 1);
                write!(sql, "EXISTS ({})", subquery_sql)
                    .unwrap_or_else(|e| panic!("Failed to write dynamic EXISTS clause: {}", e));
                self.append_subquery_params(&subquery_params, param_idx, params);
            }
            FilterExpr::NotExists {
                subquery_sql,
                subquery_params,
            } => {
                use std::fmt::Write;
                let subquery_sql =
                    rebase_subquery_sql(subquery_sql, self.db_type, *param_idx as usize - 1);
                write!(sql, "NOT EXISTS ({})", subquery_sql)
                    .unwrap_or_else(|e| panic!("Failed to write NOT EXISTS clause: {}", e));
                self.append_subquery_params(subquery_params, param_idx, params);
            }
            FilterExpr::NotExistsDynamic { subquery } => {
                use std::fmt::Write;
                let (subquery_sql, subquery_params) =
                    render_dynamic_subquery(subquery, self.db_type);
                let subquery_sql =
                    rebase_subquery_sql(&subquery_sql, self.db_type, *param_idx as usize - 1);
                write!(sql, "NOT EXISTS ({})", subquery_sql)
                    .unwrap_or_else(|e| panic!("Failed to write dynamic NOT EXISTS clause: {}", e));
                self.append_subquery_params(&subquery_params, param_idx, params);
            }
            FilterExpr::RelationExists {
                owner_table,
                owner_key,
                target_table,
                target_key,
                filter,
            } => {
                self.format_relation_exists(
                    sql,
                    *owner_table,
                    *owner_key,
                    *target_table,
                    *target_key,
                    filter.as_deref(),
                    param_idx,
                    params,
                );
            }
            FilterExpr::ThroughRelationExists {
                owner_table,
                owner_key,
                via_table,
                via_owner_key,
                via_target_key,
                target_table,
                target_key,
                filter,
            } => {
                self.format_through_relation_exists(
                    sql,
                    *owner_table,
                    *owner_key,
                    *via_table,
                    *via_owner_key,
                    *via_target_key,
                    *target_table,
                    *target_key,
                    filter.as_deref(),
                    param_idx,
                    params,
                );
            }
            FilterExpr::ExprComparison {
                left,
                operator,
                right,
            } => {
                use std::fmt::Write;
                let left_sql = left.to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    self.table_prefix.as_deref(),
                );
                let right_sql = right.to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    self.table_prefix.as_deref(),
                );
                #[cfg(feature = "sqlite")]
                let (left_sql, right_sql) = if matches!(self.db_type, DbType::Sqlite)
                    && matches!(
                        right,
                        SqlExpr::Value(Value::Decimal(_) | Value::BigDecimal(_))
                    )
                    && matches!(operator.as_str(), ">" | ">=" | "<" | "<=")
                {
                    (
                        format!("CAST({left_sql} AS NUMERIC)"),
                        format!("CAST({right_sql} AS NUMERIC)"),
                    )
                } else {
                    (left_sql, right_sql)
                };
                write!(sql, "{left_sql} {operator} {right_sql}")
                    .unwrap_or_else(|e| panic!("Failed to write expression comparison: {}", e));
            }
            FilterExpr::ExprIn { expr, values } => {
                self.format_expr_in(expr, values, false, sql, param_idx, params);
            }
            FilterExpr::ExprNotIn { expr, values } => {
                self.format_expr_in(expr, values, true, sql, param_idx, params);
            }
            FilterExpr::ExprBetween { expr, min, max } => {
                use std::fmt::Write;
                let expr_sql = expr.to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    self.table_prefix.as_deref(),
                );
                let min_sql = min.to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    self.table_prefix.as_deref(),
                );
                let max_sql = max.to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    self.table_prefix.as_deref(),
                );
                write!(sql, "{} BETWEEN {} AND {}", expr_sql, min_sql, max_sql)
                    .unwrap_or_else(|e| panic!("Failed to write expression BETWEEN clause: {}", e));
            }
            FilterExpr::ExprIsNull { expr } => {
                let expr_sql = expr.to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    self.table_prefix.as_deref(),
                );
                sql.push_str(&expr_sql);
                sql.push_str(" IS NULL");
            }
            FilterExpr::ExprIsNotNull { expr } => {
                let expr_sql = expr.to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    self.table_prefix.as_deref(),
                );
                sql.push_str(&expr_sql);
                sql.push_str(" IS NOT NULL");
            }
            FilterExpr::ExprPredicate { expr } => {
                let expr_sql = expr.to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    self.table_prefix.as_deref(),
                );
                sql.push_str(&expr_sql);
            }
            FilterExpr::TextSearch { expr, query } => {
                use std::fmt::Write;
                let expr_sql = expr.to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    self.table_prefix.as_deref(),
                );
                let query_value = crate::model::Value::Text(query.clone());
                let query_sql = crate::query::expr::SqlExpr::Value(query_value).to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    self.table_prefix.as_deref(),
                );
                let sql_fragment = match self.db_type {
                    #[cfg(feature = "postgresql")]
                    crate::DbType::PostgreSQL => format!(
                        "to_tsvector('simple', {}) @@ plainto_tsquery('simple', {})",
                        expr_sql, query_sql
                    ),
                    #[cfg(feature = "mysql")]
                    crate::DbType::MySQL => {
                        format!("MATCH({}) AGAINST ({})", expr_sql, query_sql)
                    }
                    #[cfg(feature = "sqlite")]
                    crate::DbType::Sqlite => format!("{} MATCH {}", expr_sql, query_sql),
                    #[cfg(feature = "mssql")]
                    crate::DbType::MSSQL => format!("CONTAINS({}, {})", expr_sql, query_sql),
                    #[cfg(feature = "questdb")]
                    crate::DbType::QuestDB => {
                        invalid_filter_marker("text search is not supported on QuestDB")
                    }
                    #[cfg(any(
                        feature = "duckdb",
                        feature = "clickhouse",
                        feature = "influxdb"
                    ))]
                    _ => format!("{} LIKE {}", expr_sql, query_sql),
                };
                write!(sql, "{sql_fragment}")
                    .unwrap_or_else(|e| panic!("Failed to write text search clause: {}", e));
            }
            FilterExpr::FullTextSearch(search) => {
                use std::fmt::Write;
                write!(
                    sql,
                    "{}",
                    self.full_text_search_sql(search, param_idx, params)
                )
                .unwrap_or_else(|e| panic!("Failed to write full-text search clause: {}", e));
            }
            FilterExpr::InvalidDynamicField { model, field } => {
                // 非 try_ 路径没有 Result 通道可返回错误：渲染一个必定触发
                // 数据库错误（"列不存在"）的占位引用，并把原因写进 SQL 注释，
                // 让错误以 Database 错误而不是 panic 的形式抛出（try_ 路径
                // 由 validate_filter_for_db 提前拦截并返回 Err）。
                sql.push_str(&invalid_filter_marker(&format!(
                    "Field '{field}' does not exist on model {model}"
                )));
            }
            FilterExpr::Unsupported { backend, feature } => {
                sql.push_str(&invalid_filter_marker(&format!(
                    "{feature} cannot be rendered for {backend:?}; validate the filter before formatting"
                )));
            }
        }
    }

    fn format_expr_in(
        &self,
        expr: &SqlExpr,
        values: &[SqlExpr],
        negated: bool,
        sql: &mut String,
        param_idx: &mut i32,
        params: &mut Vec<Value>,
    ) {
        let expr_sql = expr.to_sql(
            self.db_type,
            param_idx,
            params,
            self.table_prefix.as_deref(),
        );
        use std::fmt::Write;
        // 空集合短路语义：IN 恒假、NOT IN 恒真，避免生成非法的 "expr IN ()"
        if values.is_empty() {
            let truth = if negated { "1 = 1" } else { "1 = 0" };
            write!(sql, "{}", truth)
                .unwrap_or_else(|e| panic!("Failed to write expression IN clause: {}", e));
            return;
        }
        // 与 comparison_sql / format_column_values_clause 对齐：SQLite 上
        // Decimal 家族以 TEXT 存储，IN 值列表同样 CAST AS NUMERIC（任一值
        // 命中即整组转换，整数参数 CAST 无副作用），避免 TEXT 比较漏行。
        let numeric_cast = values.iter().any(|value| match value {
            SqlExpr::Value(inner) => sqlite_needs_numeric_cast(self.db_type, inner),
            _ => false,
        });
        let expr_sql = if numeric_cast {
            format!("CAST({expr_sql} AS NUMERIC)")
        } else {
            expr_sql
        };
        write!(
            sql,
            "{} {} (",
            expr_sql,
            if negated { "NOT IN" } else { "IN" }
        )
        .unwrap_or_else(|e| panic!("Failed to write expression IN clause: {}", e));
        for (idx, value) in values.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            let value_sql = value.to_sql(
                self.db_type,
                param_idx,
                params,
                self.table_prefix.as_deref(),
            );
            if numeric_cast {
                write!(sql, "CAST({value_sql} AS NUMERIC)")
                    .unwrap_or_else(|e| panic!("Failed to write IN value clause: {}", e));
            } else {
                sql.push_str(&value_sql);
            }
        }
        sql.push(')');
    }

    fn quoted_column(&self, column: &str) -> String {
        quote_column_reference(self.db_type, &self.qualified_column(column))
    }

    /// 解析值过滤列最终使用的（可能带别名前缀的）列引用。
    ///
    /// - 已含 `.` 的列视为已限定，保持原样，避免重复加前缀；
    /// - 未限定列按注册顺序匹配关联表列清单，命中则限定到该关联表别名
    ///   （多表查询中关联表过滤列必须限定到 t1/t2/t3，而不是主表 t0）；
    ///   别名为空串时渲染为裸引用（LATERAL 内层子查询只 FROM 关联表
    ///   本体、无别名，关联表列必须裸引用才能解析到内层表）；
    /// - 未命中关联表时回落到主表前缀（未设置前缀则保持原样，
    ///   单表查询行为不变）。
    fn qualified_column(&self, column: &str) -> String {
        if column.contains('.') {
            return column.to_owned();
        }
        for (alias, columns) in &self.related_tables {
            if columns.iter().any(|candidate| *candidate == column) {
                return if alias.is_empty() {
                    column.to_owned()
                } else {
                    format!("{}.{}", alias, column)
                };
            }
        }
        match self.table_prefix.as_deref() {
            Some(prefix) => format!("{}.{}", prefix, column),
            None => column.to_owned(),
        }
    }

    fn format_column_values_clause(
        &self,
        sql: &mut String,
        column: &str,
        keyword: &str,
        values: &[crate::query::filter::Value],
        param_idx: &mut i32,
        params: &mut Vec<Value>,
    ) {
        use std::fmt::Write;
        // 空集合短路语义：IN 恒假、NOT IN 恒真，避免生成非法的 "col IN ()"
        if values.is_empty() {
            let truth = if keyword == "IN" { "1 = 0" } else { "1 = 1" };
            write!(sql, "{}", truth)
                .unwrap_or_else(|e| panic!("Failed to write {} clause: {}", keyword, e));
            return;
        }
        // 与 comparison_sql 对齐：SQLite 上 Decimal 列表比较同样 CAST AS
        // NUMERIC（任一值命中即整组转换，整数参数 CAST 无副作用）。
        let numeric_cast = values
            .iter()
            .any(|value| sqlite_needs_numeric_cast(self.db_type, value));
        let col_sql = self.quoted_column(column);
        let col_sql = if numeric_cast {
            format!("CAST({col_sql} AS NUMERIC)")
        } else {
            col_sql
        };
        write!(sql, "{} {} (", col_sql, keyword)
            .unwrap_or_else(|e| panic!("Failed to write {} clause: {}", keyword, e));
        for (i, value) in values.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            let placeholder_sql = placeholder(self.db_type, *param_idx as usize);
            if numeric_cast {
                write!(sql, "CAST({placeholder_sql} AS NUMERIC)")
                    .unwrap_or_else(|e| panic!("Failed to write parameter placeholder: {}", e));
            } else {
                write!(sql, "{}", placeholder_sql)
                    .unwrap_or_else(|e| panic!("Failed to write parameter placeholder: {}", e));
            }
            params.push(value.clone().into());
            *param_idx += 1;
        }
        sql.push(')');
    }

    fn append_subquery_params(
        &self,
        subquery_params: &[Value],
        param_idx: &mut i32,
        params: &mut Vec<Value>,
    ) {
        for param in subquery_params {
            params.push(param.clone());
            *param_idx += 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn format_relation_subquery(
        &self,
        sql: &mut String,
        owner_table: &str,
        owner_key: &str,
        select_expr: &str,
        from_sql: &str,
        filter_alias: &str,
        filter: Option<&FilterExpr>,
        param_idx: &mut i32,
        params: &mut Vec<Value>,
    ) {
        use std::fmt::Write;

        let owner_column = self.outer_column(owner_table, owner_key);
        write!(
            sql,
            "{} IN (SELECT {} FROM {}",
            owner_column, select_expr, from_sql
        )
        .unwrap_or_else(|e| panic!("Failed to write relation EXISTS clause: {}", e));

        if let Some(filter) = filter {
            let filter_sql = FilterFormatter::new(self.db_type)
                .with_table_prefix(filter_alias)
                .format(filter, param_idx, params);
            sql.push_str(" WHERE ");
            sql.push_str(&filter_sql);
        }

        sql.push(')');
    }

    #[allow(clippy::too_many_arguments)]
    fn format_relation_exists(
        &self,
        sql: &mut String,
        owner_table: &str,
        owner_key: &str,
        target_table: &str,
        target_key: &str,
        filter: Option<&FilterExpr>,
        param_idx: &mut i32,
        params: &mut Vec<Value>,
    ) {
        let target_table = crate::model::quote_qualified_identifier(
            self.db_type,
            normalize_table_name_for_db(self.db_type, target_table),
        );
        let target_key = quote_column_reference(self.db_type, &format!("r0.{target_key}"));
        let from_sql = format!("{target_table} AS r0");
        self.format_relation_subquery(
            sql,
            owner_table,
            owner_key,
            &target_key,
            &from_sql,
            "r0",
            filter,
            param_idx,
            params,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn format_through_relation_exists(
        &self,
        sql: &mut String,
        owner_table: &str,
        owner_key: &str,
        via_table: &str,
        via_owner_key: &str,
        via_target_key: &str,
        target_table: &str,
        target_key: &str,
        filter: Option<&FilterExpr>,
        param_idx: &mut i32,
        params: &mut Vec<Value>,
    ) {
        let via_table = crate::model::quote_qualified_identifier(
            self.db_type,
            normalize_table_name_for_db(self.db_type, via_table),
        );
        let target_table = crate::model::quote_qualified_identifier(
            self.db_type,
            normalize_table_name_for_db(self.db_type, target_table),
        );
        let via_owner_key = quote_column_reference(self.db_type, &format!("r0.{via_owner_key}"));
        let via_target_key = quote_column_reference(self.db_type, &format!("r0.{via_target_key}"));
        let target_key = quote_column_reference(self.db_type, &format!("r1.{target_key}"));
        let from_sql = format!(
            "{via_table} AS r0 INNER JOIN {target_table} AS r1 ON {via_target_key} = {target_key}"
        );
        self.format_relation_subquery(
            sql,
            owner_table,
            owner_key,
            &via_owner_key,
            &from_sql,
            "r1",
            filter,
            param_idx,
            params,
        );
    }

    fn outer_column(&self, owner_table: &str, owner_key: &str) -> String {
        if let Some(prefix) = self.table_prefix.as_deref() {
            quote_column_reference(self.db_type, &format!("{prefix}.{owner_key}"))
        } else {
            let table = normalize_table_name_for_db(self.db_type, owner_table);
            quote_column_reference(self.db_type, &format!("{table}.{owner_key}"))
        }
    }

    /// 格式化单个比较表达式的 SQL 片段
    fn comparison_sql(
        &self,
        full_col_name: &str,
        operator: &str,
        param_idx: &i32,
        _value: &Value,
    ) -> String {
        let param_placeholder = placeholder(self.db_type, *param_idx as usize);
        #[cfg(feature = "postgresql")]
        let param_placeholder =
            if matches!(self.db_type, DbType::PostgreSQL) && self.postgresql_having_cast {
                format!("{param_placeholder}::bigint")
            } else {
                param_placeholder
            };

        #[cfg(feature = "postgresql")]
        if matches!(self.db_type, DbType::PostgreSQL) && operator == "@>" {
            return format!("{} @> ARRAY[{}]", full_col_name, param_placeholder);
        }

        // 数组成员过滤（`Vec<T>` 列的 contains）跨后端语义下推：
        // 与 SqlExpr::ArrayContains 的方言分支保持一致，避免 `@>`
        // 被原样透传给不支持该操作符的后端。
        #[cfg(feature = "mysql")]
        if matches!(self.db_type, DbType::MySQL) && operator == "@>" {
            return format!("JSON_CONTAINS({full_col_name}, JSON_ARRAY({param_placeholder}))");
        }
        #[cfg(feature = "sqlite")]
        if matches!(self.db_type, DbType::Sqlite) && operator == "@>" {
            return format!(
                "EXISTS (SELECT 1 FROM json_each({full_col_name}) AS __ormer_arr \
                 WHERE __ormer_arr.value = {param_placeholder})"
            );
        }
        #[cfg(feature = "mssql")]
        if matches!(self.db_type, DbType::MSSQL) && operator == "@>" {
            return format!(
                "EXISTS (SELECT 1 FROM OPENJSON({full_col_name}) AS __ormer_arr \
                 WHERE __ormer_arr.value = {param_placeholder})"
            );
        }
        #[cfg(feature = "duckdb")]
        if matches!(self.db_type, DbType::DuckDB) && operator == "@>" {
            return format!("list_contains({full_col_name}, {param_placeholder})");
        }
        #[cfg(feature = "clickhouse")]
        if matches!(self.db_type, DbType::ClickHouse) && operator == "@>" {
            return format!("has({full_col_name}, {param_placeholder})");
        }

        // SQLite 把 Decimal 存为 TEXT：数值比较需要显式 CAST AS NUMERIC
        // （comparison_sql / Between / IN 共用同一判定，保证同一列在不同
        // 操作符下行为一致）。
        if sqlite_needs_numeric_cast(self.db_type, _value)
            && matches!(operator, ">" | ">=" | "<" | "<=")
        {
            return format!(
                "CAST({} AS NUMERIC) {} CAST({} AS NUMERIC)",
                full_col_name, operator, param_placeholder
            );
        }

        format!("{} {} {}", full_col_name, operator, param_placeholder)
    }
}

/// 渲染层兜底：无法在 `(String, Vec<Value>)` 签名上返回错误的过滤片段
/// （InvalidDynamicField / Unsupported / 动态子查询渲染失败）统一渲染为
/// 一个不存在的占位列引用，并把原因写进 SQL 注释。
///
/// 占位引用在各后端都会在 prepare/执行期报"列不存在"错误，错误以
/// `Database` 错误而不是 panic 的形式抛出；注释中剔除 `*/` 防止用户
/// 输入提前闭合注释。
pub(crate) fn invalid_filter_marker(reason: &str) -> String {
    format!(
        "__ormer_invalid_filter__ /* ormer error: {} */",
        reason.replace("*/", "* /")
    )
}

/// 动态子查询渲染结果：失败时退化为占位引用（零占位符、零参数），
/// 保持 rust 类型收集与占位符计数对齐。
fn render_dynamic_subquery(
    subquery: &crate::query::filter::DynamicSubquery,
    db_type: DbType,
) -> (String, Vec<Value>) {
    match subquery.render(db_type) {
        Ok(rendered) => rendered,
        Err(error) => (
            invalid_filter_marker(&format!("invalid dynamic subquery: {error}")),
            Vec::new(),
        ),
    }
}

pub(crate) fn rebase_subquery_sql(sql: &str, db_type: DbType, offset: usize) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.char_indices().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut question_index = 0usize;

    while let Some((_index, ch)) = chars.next() {
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            result.push(ch);
            continue;
        }
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            result.push(ch);
            continue;
        }
        if in_single_quote || in_double_quote {
            result.push(ch);
            continue;
        }

        #[cfg(feature = "postgresql")]
        if matches!(db_type, DbType::PostgreSQL) && ch == '$' {
            let start = _index + 1;
            let mut end = start;
            while let Some((next_index, next)) = chars.peek().copied() {
                if !next.is_ascii_digit() {
                    break;
                }
                end = next_index + next.len_utf8();
                chars.next();
            }
            if end > start {
                let number = sql[start..end].parse::<usize>().unwrap_or(0);
                result.push_str(&format!("${}", number + offset));
                continue;
            }
        }

        #[cfg(feature = "mssql")]
        if matches!(db_type, DbType::MSSQL) && ch == '@' {
            if let Some((_, 'P')) = chars.peek().copied() {
                chars.next();
                let start = chars.peek().map(|(next, _)| *next).unwrap_or(sql.len());
                let mut end = start;
                while let Some((next_index, next)) = chars.peek().copied() {
                    if !next.is_ascii_digit() {
                        break;
                    }
                    end = next_index + next.len_utf8();
                    chars.next();
                }
                if end > start {
                    let number = sql[start..end].parse::<usize>().unwrap_or(0);
                    result.push_str(&format!("@P{}", number + offset));
                    continue;
                }
                result.push_str("@P");
                continue;
            }
        }

        if ch == '?' {
            question_index += 1;
            result.push_str(&crate::abstract_layer::common::common_helpers::placeholder(
                db_type,
                offset + question_index,
            ));
        } else {
            result.push(ch);
        }
    }

    result
}
