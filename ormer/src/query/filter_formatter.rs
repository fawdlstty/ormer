use crate::abstract_layer::DbType;
use crate::abstract_layer::common::common_helpers::placeholder;
use crate::model::{Value, quote_column_reference};
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
                        param_idx
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
                write!(sql, "{} IN ({})", self.quoted_column(column), subquery_sql)
                    .unwrap_or_else(|e| panic!("Failed to write subquery IN clause: {}", e));
                self.append_subquery_params(subquery_params, param_idx, params);
            }
            FilterExpr::NotInSubquery {
                column,
                subquery_sql,
                subquery_params,
            } => {
                use std::fmt::Write;
                write!(
                    sql,
                    "{} NOT IN ({})",
                    self.quoted_column(column),
                    subquery_sql
                )
                .unwrap_or_else(|e| panic!("Failed to write subquery NOT IN clause: {}", e));
                self.append_subquery_params(subquery_params, param_idx, params);
            }
            FilterExpr::And(left, right) => {
                self.format_recursive(left, sql, param_idx, params);
                sql.push_str(" AND ");
                self.format_recursive(right, sql, param_idx, params);
            }
            FilterExpr::Or(left, right) => {
                self.format_recursive(left, sql, param_idx, params);
                sql.push_str(" OR ");
                self.format_recursive(right, sql, param_idx, params);
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
                write!(sql, "EXISTS ({})", subquery_sql)
                    .unwrap_or_else(|e| panic!("Failed to write EXISTS clause: {}", e));
                self.append_subquery_params(subquery_params, param_idx, params);
            }
            FilterExpr::NotExists {
                subquery_sql,
                subquery_params,
            } => {
                use std::fmt::Write;
                write!(sql, "NOT EXISTS ({})", subquery_sql)
                    .unwrap_or_else(|e| panic!("Failed to write NOT EXISTS clause: {}", e));
                self.append_subquery_params(subquery_params, param_idx, params);
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
                write!(sql, "{} {} {}", left_sql, operator, right_sql)
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
                };
                write!(sql, "{sql_fragment}")
                    .unwrap_or_else(|e| panic!("Failed to write text search clause: {}", e));
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

    /// 格式化单个比较表达式的 SQL 片段
    fn comparison_sql(&self, full_col_name: &str, operator: &str, param_idx: &i32) -> String {
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

        format!("{} {} {}", full_col_name, operator, param_placeholder)
    }
}
