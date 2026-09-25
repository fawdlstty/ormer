#![cfg(feature = "mysql")]

use ormer::abstract_layer::common::common_helpers;
use ormer::query::insert::{InsertConflict, InsertConflictTarget};
use ormer::{DbType, OrmerError};

#[derive(Debug, ormer::Model)]
#[table = "mysql_conflict_capability_users"]
struct ConflictUser {
    #[primary]
    id: i32,
    email: String,
    name: String,
}

#[test]
fn mysql_accepts_portable_unconditional_upsert() {
    let statements = common_helpers::build_insert_statements_with_conflict::<ConflictUser>(
        DbType::MySQL,
        &[&ConflictUser {
            id: 1,
            email: "alice@example.com".to_string(),
            name: "Alice".to_string(),
        }],
        Some(&InsertConflict {
            action: Some(ormer::query::insert::InsertConflictAction::DoUpdate),
            assignments: vec![ormer::query::update::UpdateAssignment {
                column: "name".to_string(),
                value: ormer::query::update::UpdateValue::Expr(
                    ormer::query::update::UpdateExpr::IncomingColumn("name".to_string()),
                ),
            }],
            ..Default::default()
        }),
    )
    .expect("portable conflict subset renders");

    assert!(statements[0].sql.contains("ON DUPLICATE KEY UPDATE"));
    assert!(statements[0].sql.contains("name = VALUES(name)"));
}

#[test]
fn mysql_rejects_incompatible_targets_before_execution() {
    let conflict = InsertConflict {
        target: Some(InsertConflictTarget::Columns(vec!["email"])),
        action: Some(ormer::query::insert::InsertConflictAction::DoNothing),
        ..Default::default()
    };

    assert!(matches!(
        common_helpers::build_insert_statements_with_conflict::<ConflictUser>(
            DbType::MySQL,
            &[&ConflictUser {
                id: 1,
                email: "alice@example.com".to_string(),
                name: "Alice".to_string(),
            }],
            Some(&conflict),
        ),
        Err(OrmerError::UnsupportedFeature {
            backend: DbType::MySQL,
            feature: "MySQL ON DUPLICATE KEY conflict target",
        })
    ));
}
