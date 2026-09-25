#![cfg(feature = "mssql")]

use ormer::abstract_layer::common::common_helpers;
use ormer::query::insert::{InsertConflict, InsertConflictAction, InsertConflictTarget};
use ormer::query::update::{UpdateAssignment, UpdateExpr, UpdateValue};
use ormer::{DbType, OrmerError, Value};

#[derive(Debug, Clone, ormer::Model)]
#[table = "mssql_conflict_capability_users"]
struct MssqlConflictUser {
    #[primary(auto)]
    id: i32,
    #[unique]
    email: String,
    name: String,
    active: bool,
}

fn user() -> MssqlConflictUser {
    MssqlConflictUser {
        id: 0,
        email: "alice@example.com".to_string(),
        name: "Alice".to_string(),
        active: true,
    }
}

#[test]
fn mssql_insert_conflict_do_update_uses_merge() -> Result<(), Box<dyn std::error::Error>> {
    let conflict = InsertConflict {
        target: Some(InsertConflictTarget::Columns(vec!["email"])),
        action: Some(InsertConflictAction::DoUpdate),
        assignments: vec![UpdateAssignment {
            column: "name".to_string(),
            value: UpdateValue::Expr(UpdateExpr::IncomingColumn("name".to_string())),
        }],
        ..Default::default()
    };

    let statement = common_helpers::build_mssql_insert_conflict_statement::<MssqlConflictUser>(
        &[&user()],
        &conflict,
    )?;

    assert_eq!(
        statement.sql,
        "MERGE INTO mssql_conflict_capability_users AS target USING (VALUES (@P1, @P2, @P3)) AS source (email, name, active) ON target.email = source.email WHEN MATCHED THEN UPDATE SET name = source.name WHEN NOT MATCHED THEN INSERT (email, name, active) VALUES (source.email, source.name, source.active) OUTPUT inserted.id;"
    );
    assert_eq!(statement.params.len(), 3);
    assert!(matches!(statement.params[0], Value::Text(_)));
    assert_eq!(
        common_helpers::mssql_insert_returning_sql::<MssqlConflictUser>(&statement.sql),
        "MERGE INTO mssql_conflict_capability_users AS target USING (VALUES (@P1, @P2, @P3)) AS source (email, name, active) ON target.email = source.email WHEN MATCHED THEN UPDATE SET name = source.name WHEN NOT MATCHED THEN INSERT (email, name, active) VALUES (source.email, source.name, source.active) OUTPUT inserted.id, inserted.email, inserted.name, inserted.active;"
    );
    Ok(())
}

#[test]
fn mssql_insert_conflict_do_nothing_uses_merge_without_matched_action()
-> Result<(), Box<dyn std::error::Error>> {
    let conflict = InsertConflict {
        target: Some(InsertConflictTarget::Columns(vec!["email"])),
        action: Some(InsertConflictAction::DoNothing),
        ..Default::default()
    };

    let statement = common_helpers::build_mssql_insert_conflict_statement::<MssqlConflictUser>(
        &[&user()],
        &conflict,
    )?;

    assert!(!statement.sql.contains("WHEN MATCHED"));
    assert!(statement.sql.contains("WHEN NOT MATCHED THEN INSERT"));
    Ok(())
}

#[test]
fn mssql_insert_conflict_rejects_unexpressible_options() {
    let partial_target = InsertConflict {
        target: Some(InsertConflictTarget::Columns(vec!["email"])),
        target_filter: Some(ormer::FilterExpr::IsNotNull {
            column: "active".to_string(),
        }),
        action: Some(InsertConflictAction::DoNothing),
        ..Default::default()
    };
    assert!(matches!(
        common_helpers::build_mssql_insert_conflict_statement::<MssqlConflictUser>(
            &[&user()],
            &partial_target
        ),
        Err(OrmerError::UnsupportedFeature {
            backend: DbType::MSSQL,
            feature: "partial insert conflict targets",
        })
    ));

    let update_condition = InsertConflict {
        target: Some(InsertConflictTarget::Columns(vec!["email"])),
        action: Some(InsertConflictAction::DoUpdate),
        update_filter: Some(ormer::FilterExpr::IsNotNull {
            column: "active".to_string(),
        }),
        assignments: vec![UpdateAssignment {
            column: "name".to_string(),
            value: UpdateValue::Expr(UpdateExpr::IncomingColumn("name".to_string())),
        }],
        ..Default::default()
    };
    assert!(matches!(
        common_helpers::build_mssql_insert_conflict_statement::<MssqlConflictUser>(
            &[&user()],
            &update_condition
        ),
        Err(OrmerError::UnsupportedFeature {
            backend: DbType::MSSQL,
            feature: "conditional insert conflict updates",
        })
    ));

    let named_constraint = InsertConflict {
        target: Some(InsertConflictTarget::Constraint(
            "uq_mssql_conflict_users_email".to_string(),
        )),
        action: Some(InsertConflictAction::DoNothing),
        ..Default::default()
    };
    assert!(matches!(
        common_helpers::build_mssql_insert_conflict_statement::<MssqlConflictUser>(
            &[&user()],
            &named_constraint
        ),
        Err(OrmerError::UnsupportedFeature {
            backend: DbType::MSSQL,
            feature: "named insert conflict constraints",
        })
    ));
}
