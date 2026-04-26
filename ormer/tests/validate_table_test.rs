#![cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]

mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_simple!(ValidateTestUser, "validate_table_users_1");

#[cfg(any(feature = "turso", feature = "postgresql", feature = "mysql"))]
mod validate_table_tests {
    use super::*;
    use _test_common::{DbConfig, create_db_connection};

    async fn test_validate_table_success_impl(
        config: &DbConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = create_db_connection(config).await?;

        // 先删除表（如果存在）
        db.drop_table::<ValidateTestUser>().execute().await.ok();

        // 创建表
        db.create_table::<ValidateTestUser>().execute().await?;

        // 验证表结构应该成功
        db.validate_table::<ValidateTestUser>().await?;

        println!("validate_table succeeded for existing table");

        // 清理
        db.drop_table::<ValidateTestUser>().execute().await?;

        Ok(())
    }

    async fn test_validate_table_not_exists_impl(
        config: &DbConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = create_db_connection(config).await?;

        // 确保表不存在
        db.drop_table::<ValidateTestUser>().execute().await.ok();

        // 验证不存在的表应该失败
        let result = db.validate_table::<ValidateTestUser>().await;
        assert!(
            result.is_err(),
            "validate_table should fail for non-existent table"
        );

        if let Err(ormer::Error::SchemaMismatch { table, reason }) = result {
            assert_eq!(table, "validate_table_users_1");
            assert!(
                reason.contains("does not exist"),
                "Error reason should mention table does not exist"
            );
            println!("Correctly detected non-existent table: {}", reason);
        } else {
            panic!("Expected SchemaMismatch error");
        }

        Ok(())
    }

    async fn test_create_table_without_validation_impl(
        config: &DbConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = create_db_connection(config).await?;

        // 先删除表（如果存在）
        db.drop_table::<ValidateTestUser>().execute().await?;

        // 创建表（不应该进行验证）
        db.create_table::<ValidateTestUser>().execute().await?;

        println!("create_table succeeded without validation");

        // 清理
        db.drop_table::<ValidateTestUser>().execute().await?;

        Ok(())
    }

    test_on_all_dbs_result!(test_validate_table_success_impl);
    test_on_all_dbs_result!(test_validate_table_not_exists_impl);
    test_on_all_dbs_result!(test_create_table_without_validation_impl);
}
