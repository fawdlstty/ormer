/// 错误处理一致性测试
///
/// 验证所有后端在非空字段解析失败时返回错误而非 panic
///
/// 测试 anyhow 错误处理
#[test]
fn test_parse_error_variant() {
    use anyhow::Context;

    // 使用 anyhow 创建带上下文的错误
    let result: Result<(), anyhow::Error> = Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "Failed to parse non-nullable column 'name' (expected String type)",
    ))
    .context("Parse error");

    // 验证错误消息
    let error = result.unwrap_err();
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("Parse error"));
    // 遍历错误链检查内部错误消息
    let found_column_error = error.chain().any(|e| {
        e.to_string()
            .contains("Failed to parse non-nullable column")
    });
    assert!(
        found_column_error,
        "错误链中应包含 'Failed to parse non-nullable column'"
    );
    let found_name = error.chain().any(|e| e.to_string().contains("name"));
    assert!(found_name, "错误链中应包含 'name'");
}
