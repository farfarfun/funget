/// 返回可安全写进日志的 URL:抹掉用户名、密码和 query。
///
/// 这是有强制回归测试的行为 —— Python 版 `tests/test_downloader.py` 断言日志里
/// 既要出现 `https://example.com/file`,又不能出现 user/password/token/secret。
/// 下载链接里带签名、带临时凭据是常态,日志泄漏就是凭据泄漏。
pub fn safe_url(raw: &str) -> String {
    const INVALID: &str = "<invalid URL>";
    match url::Url::parse(raw) {
        Ok(parsed) => match parsed.host_str() {
            Some(host) => format!("{}://{}{}", parsed.scheme(), host, parsed.path()),
            None => INVALID.to_string(),
        },
        Err(_) => INVALID.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::safe_url;

    #[test]
    fn strips_credentials_and_query() {
        let redacted = safe_url("https://user:password@example.com/file?token=secret");
        assert_eq!(redacted, "https://example.com/file");
        for leaked in ["user", "password", "token", "secret"] {
            assert!(!redacted.contains(leaked), "日志里泄漏了 {leaked}");
        }
    }

    #[test]
    fn reports_invalid_urls_without_echoing_them() {
        // 不能把原串回显出去:非法 URL 里一样可能夹着凭据。
        assert_eq!(safe_url("not a url"), "<invalid URL>");
        assert_eq!(safe_url(""), "<invalid URL>");
    }

    #[test]
    fn keeps_plain_urls_intact() {
        assert_eq!(
            safe_url("https://example.com/a/b.bin"),
            "https://example.com/a/b.bin"
        );
    }
}
