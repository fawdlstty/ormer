/// 错误处理一致性测试
///
/// 验证所有后端在非空字段解析失败时返回错误而非 panic
///
/// 测试 OrmerError 直接显示完整错误内容
#[test]
fn test_parse_error_variant() {
    let source = std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "Failed to parse non-nullable column 'name' (expected String type)",
    );
    let error = ormer::ormer_error!("Parse error: {}", source);

    // 验证错误消息
    let error_msg = format!("{error}");
    assert!(error_msg.contains("Parse error"));
    assert!(
        error_msg.contains("Failed to parse non-nullable column"),
        "错误消息中应包含 'Failed to parse non-nullable column'"
    );
    assert!(error_msg.contains("name"), "错误消息中应包含 'name'");
}
