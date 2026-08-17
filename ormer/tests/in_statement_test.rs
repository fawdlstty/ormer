#![cfg(any(feature = "sqlite", feature = "postgresql", feature = "mysql"))]

pub mod _test_common;

// 使用宏定义测试专用模型（唯一表名）
define_test_user_direct!(TestUser, "test_in_stmt_users_1");

fn assert_in_clause(sql: &str, column: &str, expected_count: usize) {
    let needle = format!("{column} IN (");
    let start = sql.find(&needle).unwrap_or_else(|| panic!("SQL: {sql}")) + needle.len();
    let end = sql[start..]
        .find(')')
        .unwrap_or_else(|| panic!("SQL: {sql}"));
    let placeholders: Vec<&str> = sql[start..start + end].split(',').map(str::trim).collect();

    assert_eq!(placeholders.len(), expected_count, "SQL: {sql}");
    assert!(placeholders.into_iter().all(is_placeholder), "SQL: {sql}");
}

fn assert_comparison_placeholder(sql: &str, expr: &str) {
    let value = sql
        .split_once(expr)
        .unwrap_or_else(|| panic!("SQL: {sql}"))
        .1
        .trim_start()
        .split_whitespace()
        .next()
        .unwrap_or("");
    assert!(is_placeholder(value), "SQL: {sql}");
}

fn is_placeholder(value: &str) -> bool {
    value == "?"
        || value
            .strip_prefix('$')
            .is_some_and(|digits| digits.chars().all(|ch| ch.is_ascii_digit()))
        || value
            .strip_prefix("@P")
            .is_some_and(|digits| digits.chars().all(|ch| ch.is_ascii_digit()))
}

macro_rules! sql_case {
    ($label:literal, { $($setup:tt)* }, $sql:expr, $assert_sql:expr $(,)?) => {{
        $($setup)*
        let sql = $sql;

        println!("Case: {}\nSQL: {}", $label, sql);
        $assert_sql(&sql);
    }};
}

macro_rules! in_clause_case {
    ($label:literal, { $($setup:tt)* }, $sql:expr, $column:literal, $expected_count:expr $(, $extra_assert:expr)* $(,)?) => {
        sql_case!(
            $label,
            { $($setup)* },
            $sql,
            |sql: &str| {
                assert_in_clause(sql, $column, $expected_count);
                $($extra_assert(sql);)*
            },
        );
    };
}

async fn test_in_statement_cases_impl(config: &_test_common::DbConfig) {
    let _config = config; // 仅用于获取数据库类型

    // slice 类型
    in_clause_case!(
        "&[i32]",
        {
            let values: &[i32] = &[2, 4, 6, 7, 8];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.age.is_in(values))
            .to_sql(),
        "age",
        5,
        |sql: &str| assert!(sql.contains("WHERE")),
    );
    in_clause_case!(
        "&[&i32]",
        {
            let v1: &i32 = &2;
            let v2: &i32 = &4;
            let v3: &i32 = &6;
            let values: &[&i32] = &[v1, v2, v3];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.age.is_in(values))
            .to_sql(),
        "age",
        3,
    );
    in_clause_case!(
        "&[String]",
        {
            let names: &[String] = &[
                "Alice".to_string(),
                "Bob".to_string(),
                "Charlie".to_string(),
            ];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.name.is_in(names))
            .to_sql(),
        "name",
        3,
    );
    in_clause_case!(
        "&[&String]",
        {
            let names: Vec<String> = vec!["Alice".to_string(), "Bob".to_string()];
            let name_refs: Vec<&String> = names.iter().collect();
            let name_refs_slice: &[&String] = &name_refs;
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.name.is_in(name_refs_slice))
            .to_sql(),
        "name",
        2,
    );
    in_clause_case!(
        "&[&str]",
        {
            let names: &[&str] = &["Alice", "Bob", "Charlie"];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.name.is_in(names))
            .to_sql(),
        "name",
        3,
    );
    in_clause_case!(
        "in with other filters",
        {
            let values: &[i32] = &[20, 25, 30];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.age.ge(18))
            .filter(|p| p.age.is_in(values))
            .range(..10)
            .to_sql(),
        "age",
        3,
        |sql: &str| assert_comparison_placeholder(sql, "age >="),
        |sql: &str| assert!(sql.contains("LIMIT 10")),
    );
    sql_case!(
        "empty slice",
        {
            let empty_vec: &[i32] = &[];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.age.is_in(empty_vec))
            .to_sql(),
        |sql: &str| assert!(sql.contains("age IN ()")),
    );

    // Vec 类型
    in_clause_case!(
        "&Vec<i32>",
        {
            let values: Vec<i32> = vec![1, 2, 3, 4, 5];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.age.is_in(&values))
            .to_sql(),
        "age",
        5,
    );
    in_clause_case!(
        "&Vec<&i32>",
        {
            let v1 = 10;
            let v2 = 20;
            let v3 = 30;
            let values: Vec<&i32> = vec![&v1, &v2, &v3];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.age.is_in(&values))
            .to_sql(),
        "age",
        3,
    );
    in_clause_case!(
        "&Vec<String>",
        {
            let names: Vec<String> = vec!["Alice".to_string(), "Bob".to_string()];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.name.is_in(&names))
            .to_sql(),
        "name",
        2,
    );
    in_clause_case!(
        "&Vec<&String>",
        {
            let s1 = "Alice".to_string();
            let s2 = "Bob".to_string();
            let names: Vec<&String> = vec![&s1, &s2];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.name.is_in(&names))
            .to_sql(),
        "name",
        2,
    );
    in_clause_case!(
        "&Vec<&str>",
        {
            let names: Vec<&str> = vec!["Alice", "Bob", "Charlie"];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.name.is_in(&names))
            .to_sql(),
        "name",
        3,
    );

    // 数组类型
    in_clause_case!(
        "&[i32; N]",
        {
            let values: &[i32; 4] = &[1, 2, 3, 4];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.age.is_in(values))
            .to_sql(),
        "age",
        4,
    );
    in_clause_case!(
        "&[&i32; N]",
        {
            let v1 = 100;
            let v2 = 200;
            let v3 = 300;
            let values: &[&i32; 3] = &[&v1, &v2, &v3];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.age.is_in(values))
            .to_sql(),
        "age",
        3,
    );
    in_clause_case!(
        "&[String; N]",
        {
            let names: &[String; 2] = &["Alice".to_string(), "Bob".to_string()];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.name.is_in(names))
            .to_sql(),
        "name",
        2,
    );
    in_clause_case!(
        "&[&String; N]",
        {
            let s1 = "Alice".to_string();
            let s2 = "Bob".to_string();
            let names: &[&String; 2] = &[&s1, &s2];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.name.is_in(names))
            .to_sql(),
        "name",
        2,
    );
    in_clause_case!(
        "&[&str; N]",
        {
            let names: &[&str; 3] = &["Alice", "Bob", "Charlie"];
        },
        ormer::Select::<TestUser>::new()
            .filter(|p| p.name.is_in(names))
            .to_sql(),
        "name",
        3,
    );

    // 直接字面量
    in_clause_case!(
        "literal &[i32; N]",
        {},
        ormer::Select::<TestUser>::new()
            .filter(|p| {
                let values: &[i32; 5] = &[2, 4, 6, 7, 8];
                p.age.is_in(values)
            })
            .to_sql(),
        "age",
        5,
    );
    in_clause_case!(
        "literal &[&str; N]",
        {},
        ormer::Select::<TestUser>::new()
            .filter(|p| {
                let names: &[&str; 2] = &["Alice", "Bob"];
                p.name.is_in(names)
            })
            .to_sql(),
        "name",
        2,
    );
}

test_on_all_dbs!(test_in_statement_cases_impl);
