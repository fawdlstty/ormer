#![cfg(any(feature = "sqlite", feature = "postgresql"))]

//! page.md 两条能力缺口的验收测试：
//! 1. 多表/关联查询类型的 count()（同谓词 total_count，与 range() 组合一致）；
//! 2. 数组列 contains/contains_all/overlaps 的 SQL 下推与内存过滤等价，
//!    且与 range()、count() 组合时同一谓词贯穿。
//!
//! 每个测试使用独立表名：tokio 测试并行执行，共享表名会在 PG 上互相踩踏。

pub mod _test_common;

use _test_common::create_db_connection;

define_test_user!(CountUserA, "fix_count_a_users");
define_test_role!(CountRoleA, "fix_count_a_roles");
define_test_user!(CountUserB, "fix_count_b_users");
define_test_role!(CountRoleB, "fix_count_b_roles");

#[derive(Debug, ormer::Model, Clone)]
#[table = "fix_contains_a_alerts"]
struct FixAlertA {
    #[primary(auto)]
    id: i32,
    device: String,
    handlers: Vec<String>,
}

#[derive(Debug, ormer::Model, Clone)]
#[table = "fix_contains_b_alerts"]
struct FixAlertB {
    #[primary(auto)]
    id: i32,
    device: String,
    handlers: Vec<String>,
}

/// 遍历本测试覆盖的后端（sqlite + postgresql，其余按可用性）。
fn covered_configs() -> Vec<_test_common::DbConfig> {
    _test_common::get_all_db_configs()
        .into_iter()
        .filter(|(db_type, _)| {
            #[cfg(feature = "sqlite")]
            if matches!(db_type, ormer::DbType::Sqlite) {
                return true;
            }
            #[cfg(feature = "postgresql")]
            if matches!(db_type, ormer::DbType::PostgreSQL) {
                return true;
            }
            let _ = db_type;
            false
        })
        .collect()
}

/// 缺失一：同一过滤条件下 count() == 全量 fetch 后的 len。
#[tokio::test]
async fn related_count_matches_fetch_len() -> Result<(), Box<dyn std::error::Error>> {
    for config in covered_configs() {
        let db = create_db_connection(&config).await?;
        let _ = db.drop_table::<CountUserA>().execute().await;
        let _ = db.drop_table::<CountRoleA>().execute().await;
        db.create_table::<CountUserA>().execute().await?;
        db.create_table::<CountRoleA>().execute().await?;

        db.insert(&[
            CountUserA { id: 1, name: "alice".into(), age: 30, email: None },
            CountUserA { id: 2, name: "bob".into(), age: 40, email: None },
            CountUserA { id: 3, name: "carol".into(), age: 50, email: None },
        ])
        .execute()
        .await?;
        db.insert(&[
            CountRoleA { id: 1, uid: 1, name: "admin".into() },
            CountRoleA { id: 2, uid: 2, name: "admin".into() },
            CountRoleA { id: 3, uid: 3, name: "user".into() },
        ])
        .execute()
        .await?;

        // 关联表字段过滤（q.name），关联键连接条件（p.id = q.uid）
        let expected = db
            .select::<CountUserA>()
            .from::<CountRoleA>()
            .filter(|p, q| p.id.eq(q.uid))
            .filter(|_, q| q.name.eq("admin".to_string()))
            .collect::<Vec<CountUserA>>()
            .await?
            .len();

        let total = db
            .select::<CountUserA>()
            .from::<CountRoleA>()
            .filter(|p, q| p.id.eq(q.uid))
            .filter(|_, q| q.name.eq("admin".to_string()))
            .count()
            .await?;

        assert_eq!(expected, 2, "fixture rows");
        assert_eq!(
            total as usize, expected,
            "count() must equal fetched len for {:?}",
            config.0
        );
    }
    Ok(())
}

/// 缺失一：count() 与 range() 组合时谓词一致，count 不受分页影响。
#[tokio::test]
async fn related_count_ignores_range() -> Result<(), Box<dyn std::error::Error>> {
    for config in covered_configs() {
        let db = create_db_connection(&config).await?;
        let _ = db.drop_table::<CountUserB>().execute().await;
        let _ = db.drop_table::<CountRoleB>().execute().await;
        db.create_table::<CountUserB>().execute().await?;
        db.create_table::<CountRoleB>().execute().await?;

        db.insert(&[
            CountUserB { id: 1, name: "alice".into(), age: 30, email: None },
            CountUserB { id: 2, name: "bob".into(), age: 40, email: None },
        ])
        .execute()
        .await?;
        db.insert(&[
            CountRoleB { id: 1, uid: 1, name: "admin".into() },
            CountRoleB { id: 2, uid: 2, name: "admin".into() },
        ])
        .execute()
        .await?;

        let total = db
            .select::<CountUserB>()
            .from::<CountRoleB>()
            .filter(|p, q| p.id.eq(q.uid))
            .filter(|_, q| q.name.eq("admin".to_string()))
            .count()
            .await?;

        let page = db
            .select::<CountUserB>()
            .from::<CountRoleB>()
            .filter(|p, q| p.id.eq(q.uid))
            .filter(|_, q| q.name.eq("admin".to_string()))
            .range(0..1)
            .collect::<Vec<CountUserB>>()
            .await?;

        assert_eq!(total, 2, "total_count must ignore range for {:?}", config.0);
        assert_eq!(page.len(), 1, "page size must follow range");
    }
    Ok(())
}

/// 缺失二：contains 下推与内存过滤等价，并与 range/count 组合贯穿同一谓词。
#[tokio::test]
async fn array_contains_pushdown_equivalence() -> Result<(), Box<dyn std::error::Error>> {
    for config in covered_configs() {
        let db = create_db_connection(&config).await?;
        let _ = db.drop_table::<FixAlertA>().execute().await;
        db.create_table::<FixAlertA>().execute().await?;

        db.insert(&[
            FixAlertA { id: 1, device: "d1".into(), handlers: vec!["alice".into(), "bob".into()] },
            FixAlertA { id: 2, device: "d2".into(), handlers: vec!["alice".into()] },
            FixAlertA { id: 3, device: "d3".into(), handlers: vec!["carol".into()] },
        ])
        .execute()
        .await?;

        let all = db.select::<FixAlertA>().collect::<Vec<FixAlertA>>().await?;

        // contains("alice")：SQL 下推 vs 内存过滤的 id 集合一致
        let pushed: Vec<i32> = db
            .select::<FixAlertA>()
            .filter(|p| p.handlers.contains("alice"))
            .collect::<Vec<FixAlertA>>()
            .await?
            .iter()
            .map(|row| row.id)
            .collect();
        let mut in_memory: Vec<i32> = all
            .iter()
            .filter(|row| row.handlers.iter().any(|h| h == "alice"))
            .map(|row| row.id)
            .collect();
        let mut sorted_pushed = pushed.clone();
        sorted_pushed.sort();
        in_memory.sort();
        assert_eq!(
            sorted_pushed, in_memory,
            "contains pushdown mismatch on {:?}",
            config.0
        );
        assert_eq!(pushed.len(), 2);

        // 与 range() 组合：同一谓词下分页
        let page = db
            .select::<FixAlertA>()
            .filter(|p| p.handlers.contains("alice"))
            .range(0..1)
            .collect::<Vec<FixAlertA>>()
            .await?;
        assert_eq!(page.len(), 1, "page size with contains filter");

        // 与 count() 组合：同一谓词的 total_count
        let total = db
            .select::<FixAlertA>()
            .filter(|p| p.handlers.contains("alice"))
            .count(|p| p.id)
            .await?;
        assert_eq!(total, pushed.len(), "count must match filtered len");
    }
    Ok(())
}

/// 缺失二：contains_all / overlaps（any 语义）下推与内存过滤等价。
#[tokio::test]
async fn array_contains_all_any_pushdown_equivalence() -> Result<(), Box<dyn std::error::Error>> {
    for config in covered_configs() {
        let db = create_db_connection(&config).await?;
        let _ = db.drop_table::<FixAlertB>().execute().await;
        db.create_table::<FixAlertB>().execute().await?;

        db.insert(&[
            FixAlertB { id: 1, device: "d1".into(), handlers: vec!["alice".into(), "bob".into()] },
            FixAlertB { id: 2, device: "d2".into(), handlers: vec!["alice".into()] },
            FixAlertB { id: 3, device: "d3".into(), handlers: vec!["carol".into(), "bob".into()] },
        ])
        .execute()
        .await?;

        let all = db.select::<FixAlertB>().collect::<Vec<FixAlertB>>().await?;
        let ids = |rows: Vec<FixAlertB>| -> Vec<i32> {
            let mut v: Vec<i32> = rows.iter().map(|r| r.id).collect();
            v.sort();
            v
        };

        // contains_all(["alice", "bob"]) → 仅 id=1
        let all_match = db
            .select::<FixAlertB>()
            .filter(|p| p.handlers.contains_all(vec!["alice".to_string(), "bob".to_string()]))
            .collect::<Vec<FixAlertB>>()
            .await?;
        assert_eq!(
            ids(all_match),
            vec![1],
            "contains_all pushdown mismatch on {:?}",
            config.0
        );

        // overlaps(["bob", "carol"])（any 语义）→ id=1, id=3
        let any_match = db
            .select::<FixAlertB>()
            .filter(|p| p.handlers.overlaps(vec!["bob".to_string(), "carol".to_string()]))
            .collect::<Vec<FixAlertB>>()
            .await?;
        let expected_any: Vec<i32> = {
            let mut v: Vec<i32> = all
                .iter()
                .filter(|row| row.handlers.iter().any(|h| h == "bob" || h == "carol"))
                .map(|row| row.id)
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            ids(any_match),
            expected_any,
            "overlaps pushdown mismatch on {:?}",
            config.0
        );
    }
    Ok(())
}
