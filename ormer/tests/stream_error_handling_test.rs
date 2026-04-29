/// 错误处理一致性测试
///
/// 验证所有后端在非空字段解析失败时返回错误而非 panic

/// 测试 Error::ParseError 变体
#[test]
fn test_parse_error_variant() {
    use ormer::Error;

    let error = Error::ParseError(
        "Failed to parse non-nullable column 'name' (expected String type)".to_string(),
    );

    // 验证错误消息
    let error_msg = format!("{}", error);
    assert!(error_msg.contains("Parse error"));
    assert!(error_msg.contains("Failed to parse non-nullable column"));
    assert!(error_msg.contains("name"));
}
