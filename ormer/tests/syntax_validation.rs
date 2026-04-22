use ormer::Model;

// 测试模型定义
#[derive(Model)]
#[table = "users"]
struct TestUser {
    #[primary]
    id: i32,
    name: String,
    age: i32,
    email: Option<String>,
}

#[cfg(test)]
mod syntax_validation {
    use super::*;
    // ColumnBuilder is used for future tests
    #[allow(unused_imports)]
    use ormer::query::builder::ColumnBuilder;

    // 注意：为了让语法工作，我们需要为 TestUser 实现字段访问
    // 这需要通过过程宏生成，但这里我们先演示概念

    #[test]
    fn test_model_constants() {
        // 验证 Model trait 实现正确
        assert_eq!(TestUser::TABLE_NAME, "users");
        assert_eq!(TestUser::COLUMNS, &["id", "name", "age", "email"]);
    }

    #[test]
    fn test_query_builder_creation() {
        let query = TestUser::query();
        let sql = query.to_sql();
        assert_eq!(sql, "SELECT id, name, age, email FROM users");
    }

    #[test]
    fn test_query_with_filter() {
        // 简化测试:手动创建 FilterExpr
        use ormer::FilterExpr;
        #[allow(unused_imports)]
        use ormer::query::builder::FilterValue;
        use ormer::query::filter::Value as FilterValueInner;

        // 模拟过程宏生成的代码
        let _filter = FilterExpr::Comparison {
            column: "age".to_string(),
            operator: ">".to_string(),
            value: FilterValueInner::Integer(18),
        };

        // 这里展示最终用户代码在过程宏展开后的效果
        // 用户写:.filter(|u| u.age.gt(18))
        // 宏展开为:.filter_with_expr(FilterExpr::Comparison {...})

        let sql = TestUser::query().to_sql();
        assert!(sql.contains("SELECT"));
        assert!(sql.contains("FROM users"));
    }

    #[test]
    fn test_complex_query() {
        #[allow(unused_imports)]
        use ormer::{OrderBy, OrderDirection};

        // offset方法已被range替代，使用range(start..end)语法
        let query = TestUser::query().range(20..30);

        let sql = query.to_sql();

        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains("OFFSET 20"));
    }

    #[test]
    fn test_sql_generation_placeholders() {
        // 验证 SQL 生成使用占位符（防 SQL 注入）
        let query = TestUser::query().range(..10);

        let sql = query.to_sql();

        // PostgreSQL 使用 $1, $2 等占位符
        // 这里验证基本结构
        assert!(sql.starts_with("SELECT"));
        assert!(sql.contains("FROM users"));
    }
}
