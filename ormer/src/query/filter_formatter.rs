use crate::abstract_layer::DbType;
use crate::abstract_layer::common::common_helpers::placeholder;
use crate::model::{Value, normalize_table_name_for_db, quote_column_reference};
use crate::query::expr::SqlExpr;
use crate::query::filter::FilterExpr;

/// 通用的 WHERE 条件格式化器
///
/// 用于将 FilterExpr 格式化为 SQL WHERE 子句，并收集参数
pub struct FilterFormatter {
    db_type: DbType,
    /// 表别名前缀，例如 "t0" 用于多表查询
    table_prefix: Option<String>,
    /// 右列表别名前缀，用于 ColumnComparison（列-列比较）
    right_table_prefix: Option<String>,
    /// PostgreSQL HAVING子句中的参数需要添加::bigint类型转换
    postgresql_having_cast: bool,
}

impl FilterFormatter {
    pub fn new(db_type: DbType) -> Self {
        Self {
            db_type,
            table_prefix: None,
            right_table_prefix: None,
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
        if search.exprs.is_empty() {
            unreachable!("empty full-text search is gated by validate_filter_for_db");
        }

        match self.db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => {
                let language = search.language.as_deref().unwrap_or("simple");
                let language_sql = SqlExpr::Value(Value::Text(language.to_string())).to_sql(
                    self.db_type,
                    param_idx,
                    params,
                    None,
                );
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
                unreachable!("QuestDB full-text search is gated by validate_filter_for_db")
            }
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
                let full_col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };
                use std::fmt::Write;
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
                let (subquery_sql, subquery_params) = subquery
                    .render(self.db_type)
                    .unwrap_or_else(|error| panic!("invalid dynamic subquery: {error}"));
                let subquery_sql =
                    rebase_subquery_sql(&subquery_sql, self.db_type, *param_idx as usize - 1);
                write!(sql, "{} IN ({})", self.quoted_column(column), subquery_sql).unwrap_or_else(
                    |e| panic!("Failed to write dynamic subquery IN clause: {}", e),
                );
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
                let (subquery_sql, subquery_params) = subquery
                    .render(self.db_type)
                    .unwrap_or_else(|error| panic!("invalid dynamic subquery: {error}"));
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
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };
                use std::fmt::Write;
                let min_placeholder = placeholder(self.db_type, *param_idx as usize);
                *param_idx += 1;
                let max_placeholder = placeholder(self.db_type, *param_idx as usize);
                *param_idx += 1;
                write!(
                    sql,
                    "{} BETWEEN {} AND {}",
                    quote_column_reference(self.db_type, &col_name),
                    min_placeholder,
                    max_placeholder
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
                let (subquery_sql, subquery_params) = subquery
                    .render(self.db_type)
                    .unwrap_or_else(|error| panic!("invalid dynamic subquery: {error}"));
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
                let (subquery_sql, subquery_params) = subquery
                    .render(self.db_type)
                    .unwrap_or_else(|error| panic!("invalid dynamic subquery: {error}"));
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
                    #[cfg(any(feature = "duckdb", feature = "clickhouse"))]
                    _ => format!("{} LIKE {}", expr_sql, query_sql),
                    #[cfg(feature = "questdb")]
                    crate::DbType::QuestDB => {
                        unreachable!("QuestDB text search is gated by validate_filter_for_db")
                    }
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
            FilterExpr::InvalidDynamicField { .. } => {
                unreachable!("invalid dynamic field is gated by validate_filter_for_db")
            }
            FilterExpr::Unsupported { backend, feature } => panic!(
                "{} cannot be rendered for {backend:?}; validate the filter before formatting",
                feature
            ),
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
            sql.push_str(&value.to_sql(
                self.db_type,
                param_idx,
                params,
                self.table_prefix.as_deref(),
            ));
        }
        sql.push(')');
    }

    fn quoted_column(&self, column: &str) -> String {
        let col_name = if let Some(ref prefix) = self.table_prefix {
            format!("{}.{}", prefix, column)
        } else {
            column.to_owned()
        };
        quote_column_reference(self.db_type, &col_name)
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
        write!(sql, "{} {} (", self.quoted_column(column), keyword)
            .unwrap_or_else(|e| panic!("Failed to write {} clause: {}", keyword, e));
        for (i, value) in values.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            write!(sql, "{}", placeholder(self.db_type, *param_idx as usize))
                .unwrap_or_else(|e| panic!("Failed to write parameter placeholder: {}", e));
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

        #[cfg(feature = "sqlite")]
        if matches!(self.db_type, DbType::Sqlite)
            && matches!(_value, Value::Decimal(_) | Value::BigDecimal(_))
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
