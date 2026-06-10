use crate::abstract_layer::DbType;
use crate::model::Value;
use crate::query::filter::{FilterExpr, Value as FilterValue};

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
                    self.comparison_sql(&full_col_name, operator, param_idx)
                )
                .unwrap_or_else(|e| panic!("Failed to write SQL WHERE clause: {}", e));

                // 转换 filter Value 到 ormer Value
                let ormer_value = Self::convert_filter_value(value);
                params.push(ormer_value);
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
                write!(sql, "{} {} {}", left_col, operator, right_col)
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
                write!(sql, "{} IN (", col_name)
                    .unwrap_or_else(|e| panic!("Failed to write IN clause: {}", e));
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(", ");
                    }
                    write!(sql, "{}", self.in_placeholder(param_idx))
                        .unwrap_or_else(|e| panic!("Failed to write parameter placeholder: {}", e));
                    // 转换 filter Value 到 ormer Value
                    let ormer_value = Self::convert_filter_value(value);
                    params.push(ormer_value);
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
                write!(sql, "{} NOT IN (", col_name)
                    .unwrap_or_else(|e| panic!("Failed to write NOT IN clause: {}", e));
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(", ");
                    }
                    write!(sql, "{}", self.in_placeholder(param_idx))
                        .unwrap_or_else(|e| panic!("Failed to write parameter placeholder: {}", e));
                    // 转换 filter Value 到 ormer Value
                    let ormer_value = Self::convert_filter_value(value);
                    params.push(ormer_value);
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
                write!(sql, "{} IN ({})", col_name, subquery_sql)
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
                write!(sql, "{} NOT IN ({})", col_name, subquery_sql)
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
                write!(sql, "{} IS NULL", col_name)
                    .unwrap_or_else(|e| panic!("Failed to write IS NULL clause: {}", e));
            }
            FilterExpr::IsNotNull { column } => {
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };
                use std::fmt::Write;
                write!(sql, "{} IS NOT NULL", col_name)
                    .unwrap_or_else(|e| panic!("Failed to write IS NOT NULL clause: {}", e));
            }
            FilterExpr::Between { column, min, max } => {
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };
                use std::fmt::Write;
                let min_placeholder = self.between_placeholder(param_idx);
                *param_idx += 1;
                let max_placeholder = self.between_placeholder(param_idx);
                *param_idx += 1;
                write!(
                    sql,
                    "{} BETWEEN {} AND {}",
                    col_name, min_placeholder, max_placeholder
                )
                .unwrap_or_else(|e| panic!("Failed to write BETWEEN clause: {}", e));
                let ormer_min = Self::convert_filter_value(min);
                let ormer_max = Self::convert_filter_value(max);
                params.push(ormer_min);
                params.push(ormer_max);
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
    fn comparison_sql(&self, full_col_name: &str, operator: &str, _param_idx: &i32) -> String {
        match self.db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => {
                let param_placeholder = if self.postgresql_having_cast {
                    "$".to_string() + &_param_idx.to_string() + "::bigint"
                } else {
                    "$".to_string() + &_param_idx.to_string()
                };
                format!("{} {} {}", full_col_name, operator, param_placeholder)
            }
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => format!("{} {} ?", full_col_name, operator),
            #[cfg(feature = "mysql")]
            DbType::MySQL => format!("{} {} ?", full_col_name, operator),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => format!("{} {} @P", full_col_name, operator),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            _ => panic!("No database backend available"),
        }
    }

    /// 格式化 IN 子句的单个占位符
    fn in_placeholder(&self, _param_idx: &i32) -> String {
        match self.db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => "$".to_string() + &_param_idx.to_string(),
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "?".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "?".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "@P".to_string(),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            _ => panic!("No database backend available"),
        }
    }

    /// 格式化 BETWEEN 子句的占位符
    fn between_placeholder(&self, _param_idx: &i32) -> String {
        match self.db_type {
            #[cfg(feature = "postgresql")]
            DbType::PostgreSQL => "$".to_string() + &_param_idx.to_string(),
            #[cfg(feature = "sqlite")]
            DbType::Sqlite => "?".to_string(),
            #[cfg(feature = "mysql")]
            DbType::MySQL => "?".to_string(),
            #[cfg(feature = "mssql")]
            DbType::MSSQL => "@P".to_string(),
            #[cfg(not(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "mssql"
            )))]
            _ => panic!("No database backend available"),
        }
    }

    /// 转换 filter::Value 到 model::Value
    fn convert_filter_value(value: &FilterValue) -> Value {
        match value {
            FilterValue::Integer(v) => Value::Integer(*v),
            FilterValue::BigInt(v) => Value::BigInt(*v),
            FilterValue::Text(v) => Value::Text(v.clone()),
            FilterValue::Real(v) => Value::Real(*v),
            FilterValue::Boolean(v) => Value::Boolean(*v),
            FilterValue::Bytes(v) => Value::Bytes(v.clone()),
            FilterValue::DateTime(v) => Value::DateTime(*v),
            FilterValue::Json(v) => Value::Json(v.clone()),
            FilterValue::Uuid(v) => Value::Uuid(*v),
            FilterValue::Null => Value::Null,
        }
    }
}
