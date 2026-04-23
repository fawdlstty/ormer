use ormer::Model;

#[derive(Debug, Model)]
#[table = "sq_users"]
struct TestUser {
    #[primary(auto)]
    id: i32,
    name: String,
    age: i32,
}

#[derive(Debug, Model)]
#[table = "sq_roles"]
struct TestRole {
    #[primary(auto)]
    id: i32,
    #[foreign(TestUser)]
    uid: i32,
    name: String,
}

// ==================== MappedSelect SQL 生成测试 ====================

#[test]
fn test_mapped_select_basic() {
    // 测试基本的 map_to SQL 生成
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.name.eq("Alice"))
        .map_to(|u| u.id)
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.starts_with("SELECT id FROM sq_users"));
    assert!(sql.contains("WHERE name = ?"));
}

#[test]
fn test_mapped_select_different_column() {
    // 测试映射到不同字段
    let sql = ormer::Select::<TestRole>::new()
        .filter(|r| r.name.eq("admin"))
        .map_to(|r| r.uid)
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.starts_with("SELECT uid FROM sq_roles"));
    assert!(sql.contains("WHERE name = ?"));
}

#[test]
fn test_mapped_select_without_filter() {
    // 测试不带过滤条件的 map_to
    let sql = ormer::Select::<TestUser>::new().map_to(|u| u.name).to_sql();

    println!("SQL: {}", sql);
    assert!(sql.starts_with("SELECT name FROM sq_users"));
    assert!(!sql.contains("WHERE"));
}

#[test]
fn test_mapped_select_with_order_by() {
    // 测试带排序的 map_to
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.ge(18))
        .order_by(|u| u.name)
        .map_to(|u| u.id)
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.starts_with("SELECT id FROM sq_users"));
    assert!(sql.contains("WHERE age >= ?"));
    assert!(sql.contains("ORDER BY name ASC"));
}

#[test]
fn test_mapped_select_with_range() {
    // 测试带范围限制的 map_to
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.ge(18))
        .range(..10)
        .map_to(|u| u.id)
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.starts_with("SELECT id FROM sq_users"));
    assert!(sql.contains("WHERE age >= ?"));
    assert!(sql.contains("LIMIT 10"));
}

#[test]
fn test_mapped_select_with_order_and_range() {
    // 测试带排序和范围的 map_to
    let sql = ormer::Select::<TestUser>::new()
        .order_by_desc(|u| u.age)
        .range(5..15)
        .map_to(|u| u.name)
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.starts_with("SELECT name FROM sq_users"));
    assert!(sql.contains("ORDER BY age DESC"));
    assert!(sql.contains("LIMIT 10 OFFSET 5"));
}

#[test]
fn test_mapped_select_multiple_filters() {
    // 测试带多个过滤条件的 map_to
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.name.eq("Alice"))
        .filter(|u| u.age.ge(18))
        .map_to(|u| u.id)
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.starts_with("SELECT id FROM sq_users"));
    assert!(sql.contains("name = ?"));
    assert!(sql.contains("age >= ?"));
    assert!(sql.contains(" AND "));
}

// ==================== MappedSelect Clone 测试 ====================

#[test]
fn test_mapped_select_clone() {
    // 测试 MappedSelect 可以被克隆
    let select = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.ge(18))
        .map_to(|u| u.id);

    let cloned = select.clone();

    let sql1 = select.to_sql();
    let sql2 = cloned.to_sql();

    assert_eq!(sql1, sql2);
}

#[test]
fn test_mapped_select_clone_with_filters() {
    // 测试带过滤条件的 MappedSelect 克隆
    let select = ormer::Select::<TestUser>::new()
        .filter(|u| u.name.eq("Alice"))
        .filter(|u| u.age.ge(18))
        .order_by(|u| u.name)
        .range(..5)
        .map_to(|u| u.id);

    let cloned = select.clone();

    assert_eq!(select.to_sql(), cloned.to_sql());
}

// ==================== 子查询 SQL 生成测试 ====================

#[test]
fn test_subquery_sql_generation() {
    // 测试子查询的 SQL 生成

    // 构建子查询
    let subquery = ormer::Select::<TestRole>::new()
        .filter(|r| r.name.eq("admin"))
        .map_to(|r| r.uid);

    // 生成子查询的 SQL
    let subquery_sql = subquery.to_sql();
    println!("Subquery SQL: {}", subquery_sql);

    assert!(subquery_sql.starts_with("SELECT uid FROM sq_roles"));
    assert!(subquery_sql.contains("WHERE name = ?"));
}

#[test]
fn test_subquery_sql_with_multiple_filters() {
    // 测试带多个过滤条件的子查询 SQL
    let subquery = ormer::Select::<TestRole>::new()
        .filter(|r| r.name.eq("admin"))
        .filter(|r| r.uid.ge(10))
        .map_to(|r| r.uid);

    let subquery_sql = subquery.to_sql();
    println!("Subquery SQL: {}", subquery_sql);

    assert!(subquery_sql.starts_with("SELECT uid FROM sq_roles"));
    assert!(subquery_sql.contains("name = ?"));
    assert!(subquery_sql.contains("uid >= ?"));
    assert!(subquery_sql.contains(" AND "));
}

#[test]
fn test_subquery_sql_without_filter() {
    // 测试不带过滤条件的子查询 SQL
    let subquery = ormer::Select::<TestRole>::new().map_to(|r| r.uid);

    let subquery_sql = subquery.to_sql();
    println!("Subquery SQL: {}", subquery_sql);

    assert!(subquery_sql.starts_with("SELECT uid FROM sq_roles"));
    assert!(!subquery_sql.contains("WHERE"));
}

#[test]
fn test_subquery_sql_with_order_and_range() {
    // 测试带排序和范围的子查询 SQL
    let subquery = ormer::Select::<TestRole>::new()
        .filter(|r| r.name.eq("admin"))
        .order_by(|r| r.uid)
        .range(..10)
        .map_to(|r| r.uid);

    let subquery_sql = subquery.to_sql();
    println!("Subquery SQL: {}", subquery_sql);

    assert!(subquery_sql.starts_with("SELECT uid FROM sq_roles"));
    assert!(subquery_sql.contains("WHERE name = ?"));
    assert!(subquery_sql.contains("ORDER BY uid ASC"));
    assert!(subquery_sql.contains("LIMIT 10"));
}

// ==================== 不同字段类型测试 ====================

#[test]
fn test_mapped_select_string_field() {
    // 测试映射到字符串字段
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.age.eq(25))
        .map_to(|u| u.name)
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.starts_with("SELECT name FROM sq_users"));
    assert!(sql.contains("WHERE age = ?"));
}

#[test]
fn test_mapped_select_age_field() {
    // 测试映射到年龄字段
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.name.eq("Alice"))
        .map_to(|u| u.age)
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.starts_with("SELECT age FROM sq_users"));
    assert!(sql.contains("WHERE name = ?"));
}

// ==================== IsInValues Trait 测试 ====================

#[test]
fn test_mapped_select_is_in_values_trait() {
    // 测试 MappedSelect 实现了 IsInValues trait
    // 验证 trait 方法可以被调用

    use ormer::query::builder::IsInValues;

    let subquery = ormer::Select::<TestRole>::new()
        .filter(|r| r.name.eq("admin"))
        .map_to(|r| r.uid);

    // 转换为 WHERE 表达式
    let where_expr = subquery.to_in_expr("uid".to_string());

    // 验证表达式包含子查询 SQL
    // 这里我们主要验证 trait 实现可以编译通过
    // 实际的 SQL 生成在其他测试中验证
    drop(where_expr);
}

// ==================== 复杂查询场景测试 ====================

#[test]
fn test_mapped_select_complex_scenario() {
    // 测试复杂查询场景：多过滤 + 排序 + 范围
    let sql = ormer::Select::<TestUser>::new()
        .filter(|u| u.name.eq("Alice"))
        .filter(|u| u.age.ge(18))
        .filter(|u| u.age.le(65))
        .order_by_desc(|u| u.age)
        .range(10..20)
        .map_to(|u| u.id)
        .to_sql();

    println!("SQL: {}", sql);
    assert!(sql.starts_with("SELECT id FROM sq_users"));
    assert!(sql.contains("name = ?"));
    assert!(sql.contains("age >= ?"));
    assert!(sql.contains("age <= ?"));
    assert!(sql.contains("ORDER BY age DESC"));
    assert!(sql.contains("LIMIT 10 OFFSET 10"));
}

#[test]
fn test_subquery_for_in_clause() {
    // 测试用于 IN 子句的子查询 SQL 生成
    // 这验证了子查询可以正确生成 SQL

    let subquery = ormer::Select::<TestRole>::new()
        .filter(|r| r.name.eq("admin"))
        .map_to(|r| r.uid);

    let sql = subquery.to_sql();
    println!("Subquery SQL for IN: {}", sql);

    // 验证生成的 SQL 适合作为 IN 子句的子查询
    assert!(sql.starts_with("SELECT"));
    assert!(sql.contains("uid"));
    assert!(sql.contains("FROM sq_roles"));
}
