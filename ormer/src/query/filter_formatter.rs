use crate::abstract_layer::DbType;
use crate::abstract_layer::common::common_helpers::placeholder;
use crate::model::{Value, quote_column_reference};
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
                // 生成 IN 语句: column IN (?, ?, ...)
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };

                use std::fmt::Write;
                write!(
                    sql,
                    "{} IN (",
                    quote_column_reference(self.db_type, &col_name)
                )
                .unwrap_or_else(|e| panic!("Failed to write IN clause: {}", e));
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
            FilterExpr::NotIn { column, values } => {
                // 生成 NOT IN 语句: column NOT IN (?, ?, ...)
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };

                use std::fmt::Write;
                write!(
                    sql,
                    "{} NOT IN (",
                    quote_column_reference(self.db_type, &col_name)
                )
                .unwrap_or_else(|e| panic!("Failed to write NOT IN clause: {}", e));
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
            FilterExpr::InSubquery {
                column,
                subquery_sql,
                subquery_params,
            } => {
                // 生成子查询 IN 语句: column IN (SELECT ...)
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };

                use std::fmt::Write;
                write!(
                    sql,
                    "{} IN ({})",
                    quote_column_reference(self.db_type, &col_name),
                    subquery_sql
                )
                .unwrap_or_else(|e| panic!("Failed to write subquery IN clause: {}", e));

                // 添加子查询的参数
                for param in subquery_params {
                    params.push(param.clone());
                    *param_idx += 1;
                }
            }
            FilterExpr::NotInSubquery {
                column,
                subquery_sql,
                subquery_params,
            } => {
                // 生成子查询 NOT IN 语句: column NOT IN (SELECT ...)
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };

                use std::fmt::Write;
                write!(
                    sql,
                    "{} NOT IN ({})",
                    quote_column_reference(self.db_type, &col_name),
                    subquery_sql
                )
                .unwrap_or_else(|e| panic!("Failed to write subquery NOT IN clause: {}", e));

                // 添加子查询的参数
                for param in subquery_params {
                    params.push(param.clone());
                    *param_idx += 1;
                }
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
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };
                use std::fmt::Write;
                write!(
                    sql,
                    "{} IS NULL",
                    quote_column_reference(self.db_type, &col_name)
                )
                .unwrap_or_else(|e| panic!("Failed to write IS NULL clause: {}", e));
            }
            FilterExpr::IsNotNull { column } => {
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };
                use std::fmt::Write;
                write!(
                    sql,
                    "{} IS NOT NULL",
                    quote_column_reference(self.db_type, &col_name)
                )
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
                for param in subquery_params {
                    params.push(param.clone());
                    *param_idx += 1;
                }
            }
            FilterExpr::NotExists {
                subquery_sql,
                subquery_params,
            } => {
                use std::fmt::Write;
                write!(sql, "NOT EXISTS ({})", subquery_sql)
                    .unwrap_or_else(|e| panic!("Failed to write NOT EXISTS clause: {}", e));
                for param in subquery_params {
                    params.push(param.clone());
                    *param_idx += 1;
                }
            }
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
