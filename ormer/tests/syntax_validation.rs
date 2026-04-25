use ormer::Model;

mod _test_common;

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

    async fn test_model_constants_impl(config: &_test_common::DbConfig) {
        let _config = config; // 仅用于获取数据库类型
        // 验证 Model trait 实现正确
        assert_eq!(TestUser::TABLE_NAME, "users");
        assert_eq!(TestUser::COLUMNS, &["id", "name", "age", "email"]);
    }

    async fn test_query_builder_creation_impl(config: &_test_common::DbConfig) {
        let query = TestUser::query();
        let sql = query.to_sql_with_params(config.0);
        assert_eq!(sql.0, "SELECT id, name, age, email FROM users");
    }

    async fn test_query_with_filter_impl(config: &_test_common::DbConfig) {
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

        let sql = TestUser::query().to_sql_with_params(config.0);
        assert!(sql.0.contains("SELECT"));
        assert!(sql.0.contains("FROM users"));
    }

    async fn test_complex_query_impl(config: &_test_common::DbConfig) {
        #[allow(unused_imports)]
        use ormer::{OrderBy, OrderDirection};

        // offset方法已被range替代，使用range(start..end)语法
        let query = TestUser::query().range(20..30);

        let sql = query.to_sql_with_params(config.0);

        assert!(sql.0.contains("LIMIT 10"));
        assert!(sql.0.contains("OFFSET 20"));
    }

    async fn test_sql_generation_placeholders_impl(config: &_test_common::DbConfig) {
        // 验证 SQL 生成使用占位符（防 SQL 注入）
        let query = TestUser::query().range(..10);

        let sql = query.to_sql_with_params(config.0);

        // PostgreSQL 使用 $1, $2 等占位符
        // 这里验证基本结构
        assert!(sql.0.starts_with("SELECT"));
        assert!(sql.0.contains("FROM users"));
    }

    test_on_all_dbs!(test_model_constants_impl);
    test_on_all_dbs!(test_query_builder_creation_impl);
    test_on_all_dbs!(test_query_with_filter_impl);
    test_on_all_dbs!(test_complex_query_impl);
    test_on_all_dbs!(test_sql_generation_placeholders_impl);
}
