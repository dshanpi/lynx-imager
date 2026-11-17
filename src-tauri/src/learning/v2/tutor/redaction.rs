//! Conservative line redaction before truncation, including diff-prefixed PEM blocks.
use regex::Regex;
use std::sync::LazyLock;

static SECRET_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r#"(?i)(?:authorization\s*:\s*bearer\b|(?:api[_ -]?key|password|private[_ -]?key|(?:(?:access|refresh)[_ -]?)?token)["']?\s*[:=]|\bsk-[A-Za-z0-9_-]{8,}\b|https?://[^/\s:@]+:[^/\s@]+@)"#
).expect("static tutor redaction pattern")
});

pub(crate) fn clean(value: &str) -> String {
    redact(value).chars().take(4000).collect()
}

pub(crate) fn task_log(value: &str) -> String {
    let redacted = redact(value);
    let mut start = redacted.len().saturating_sub(32768);
    while !redacted.is_char_boundary(start) {
        start += 1;
    }
    redacted[start..].into()
}

fn redact(value: &str) -> String {
    let mut result = String::new();
    let mut private_key = false;
    for line in value.lines() {
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
            private_key = true;
            result.push_str("[REDACTED PRIVATE KEY]\n");
        }
        if private_key {
            if line.contains("-----END") && line.contains("PRIVATE KEY-----") {
                private_key = false;
            }
            continue;
        }
        if SECRET_LINE.is_match(line) {
            result.push_str("[REDACTED CREDENTIAL LINE]");
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    result.trim_end_matches('\n').into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_tail_is_utf8_byte_bounded_without_changing_ai_budget() {
        let text = format!("{}\nfinal diagnostic", "诊断".repeat(10000));
        let result = task_log(&text);
        assert!(result.ends_with("final diagnostic"));
        assert!(result.len() <= 32768);
        assert_eq!(clean(&text).chars().count(), 4000);
    }
    #[test]
    fn diffs_and_logs_redact_credentials_before_truncation() {
        let text = "+int answer = 42;\n+password = 'canary-password';\n+{\"api_key\": \"canary-json\"}\n+https://user:canary-url@example.invalid\n+-----BEGIN OPENSSH PRIVATE KEY-----\n+canary-key\n+-----END OPENSSH PRIVATE KEY-----\n compiler failed\n";
        let result = clean(text);
        assert!(result.contains("answer = 42"));
        assert!(result.contains("compiler failed"));
        assert!(!result.contains("canary"));
        assert!(!clean(&format!("{} password=canary-end", "x".repeat(4500))).contains('x'));
        assert!(!clean("-----BEGIN RSA PRIVATE KEY-----\ncanary-no-end").contains("canary"));
    }
}
