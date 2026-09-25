#![cfg(any(feature = "mysql", feature = "mssql"))]

use ormer::DbType;
#[cfg(feature = "mysql")]
use ormer::OrmerError;
#[cfg(feature = "mssql")]
use ormer::abstract_layer::common::common_helpers;

#[derive(Debug, Clone, ormer::Model)]
#[table = "returning_capability_users"]
struct ReturningCapabilityUser {
    #[primary(auto)]
    id: i32,
    name: String,
    active: bool,
}

#[test]
#[cfg(feature = "mssql")]
fn mssql_insert_returning_replaces_auto_increment_output() -> Result<(), Box<dyn std::error::Error>>
{
    let user = ReturningCapabilityUser {
        id: 0,
        name: "Alice".to_string(),
        active: true,
    };
    let (sql, params) = common_helpers::build_insert_statement_with_auto_increment_returning::<
        ReturningCapabilityUser,
    >(DbType::MSSQL, &[&user])?;

    assert_eq!(
        sql,
        "INSERT INTO returning_capability_users (name, active) OUTPUT inserted.id VALUES (@P1, @P2)"
    );
    assert_eq!(params.len(), 2);
    assert_eq!(
        common_helpers::mssql_insert_returning_sql::<ReturningCapabilityUser>(&sql),
        "INSERT INTO returning_capability_users (name, active) OUTPUT inserted.id, inserted.name, inserted.active VALUES (@P1, @P2)"
    );
    Ok(())
}

#[test]
#[cfg(feature = "mssql")]
fn mssql_update_and_delete_returning_use_output_sources() {
    assert_eq!(
        common_helpers::mssql_update_returning_sql::<ReturningCapabilityUser>(
            "UPDATE returning_capability_users SET name = @P1 WHERE id = @P2"
        ),
        "UPDATE returning_capability_users SET name = @P1 OUTPUT inserted.id, inserted.name, inserted.active WHERE id = @P2"
    );
    assert_eq!(
        common_helpers::mssql_update_returning_sql::<ReturningCapabilityUser>(
            "UPDATE target SET target.name = source.name FROM returning_capability_users AS target"
        ),
        "UPDATE target SET target.name = source.name OUTPUT inserted.id, inserted.name, inserted.active FROM returning_capability_users AS target"
    );
    assert_eq!(
        common_helpers::mssql_delete_returning_sql::<ReturningCapabilityUser>(
            "DELETE FROM returning_capability_users WHERE id = @P1"
        ),
        "DELETE FROM returning_capability_users OUTPUT deleted.id, deleted.name, deleted.active WHERE id = @P1"
    );
}

#[tokio::test]
#[cfg(feature = "mysql")]
async fn mysql_returning_is_unsupported_feature() -> Result<(), Box<dyn std::error::Error>> {
    let db = ormer::Database::connect(
        DbType::MySQL,
        "mysql://root:password@127.0.0.1:3306/ormer_test",
    )
    .await?;
    let user = ReturningCapabilityUser {
        id: 0,
        name: "Alice".to_string(),
        active: true,
    };

    let error = db
        .insert(&user)
        .returning()
        .await
        .expect_err("MySQL insert returning must be capability gated");
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::MySQL,
            feature: "DML RETURNING",
        }
    ));

    let error = db
        .update::<ReturningCapabilityUser>()
        .set(|user| user.name = user.name.set("Bob".to_string()))
        .returning()
        .await
        .expect_err("MySQL update returning must be capability gated");
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::MySQL,
            feature: "DML RETURNING",
        }
    ));

    let error = db
        .delete::<ReturningCapabilityUser>()
        .returning()
        .await
        .expect_err("MySQL delete returning must be capability gated");
    assert!(matches!(
        error,
        OrmerError::UnsupportedFeature {
            backend: DbType::MySQL,
            feature: "DML RETURNING",
        }
    ));

    Ok(())
}
