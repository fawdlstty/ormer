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
}

impl FilterFormatter {
    pub fn new(db_type: DbType) -> Self {
        Self {
            db_type,
            table_prefix: None,
            right_table_prefix: None,
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
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };

                match self.db_type {
                    #[cfg(feature = "postgresql")]
                    DbType::PostgreSQL => {
                        use std::fmt::Write;
                        write!(sql, "{} {} ${}", col_name, operator, param_idx).unwrap();
                    }
                    #[cfg(feature = "turso")]
                    DbType::Turso => {
                        use std::fmt::Write;
                        write!(sql, "{} {} ?", col_name, operator).unwrap();
                    }
                    #[cfg(feature = "mysql")]
                    DbType::MySQL => {
                        use std::fmt::Write;
                        write!(sql, "{} {} ?", col_name, operator).unwrap();
                    }
                }
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
                write!(sql, "{} {} {}", left_col, operator, right_col).unwrap();
            }
            FilterExpr::In { column, values } => {
                // 生成 IN 语句: column IN (?, ?, ...)
                let col_name = if let Some(ref prefix) = self.table_prefix {
                    format!("{}.{}", prefix, column)
                } else {
                    column.clone()
                };

                use std::fmt::Write;
                write!(sql, "{} IN (", col_name).unwrap();
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(", ");
                    }
                    match self.db_type {
                        #[cfg(feature = "postgresql")]
                        DbType::PostgreSQL => {
                            write!(sql, "${}", param_idx).unwrap();
                        }
                        #[cfg(feature = "turso")]
                        DbType::Turso => {
                            sql.push('?');
                        }
                        #[cfg(feature = "mysql")]
                        DbType::MySQL => {
                            sql.push('?');
                        }
                    }
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
                write!(sql, "{} IN ({})", col_name, subquery_sql).unwrap();

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
        }
    }

    /// 转换 filter::Value 到 model::Value
    fn convert_filter_value(value: &FilterValue) -> Value {
        match value {
            FilterValue::Integer(v) => Value::Integer(*v),
            FilterValue::Text(v) => Value::Text(v.clone()),
            FilterValue::Real(v) => Value::Real(*v),
            FilterValue::Null => Value::Null,
        }
    }
}
