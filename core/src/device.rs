//! GitHub cihaz koduyla giriş (device flow): token isteğine gelen yanıtın anlamı.

#[derive(Debug, PartialEq, Eq)]
pub enum Poll {
    Token(String),
    /// Kullanıcı henüz onaylamadı
    Pending,
    /// Çok sık soruldu: aralığı 5 saniye uzat
    SlowDown,
    Failed(String),
}

pub fn parse_poll(json: &str) -> Poll {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Poll::Failed("GitHub'dan beklenmeyen yanıt".into());
    };
    if let Some(t) = v.get("access_token").and_then(|t| t.as_str()).filter(|t| !t.is_empty()) {
        return Poll::Token(t.to_string());
    }
    match v.get("error").and_then(|e| e.as_str()) {
        Some("authorization_pending") => Poll::Pending,
        Some("slow_down") => Poll::SlowDown,
        Some("expired_token") => Poll::Failed("kodun süresi doldu, yeniden deneyin".into()),
        Some("access_denied") => Poll::Failed("giriş reddedildi".into()),
        Some(other) => Poll::Failed(format!("giriş başarısız: {other}")),
        None => Poll::Failed("GitHub'dan beklenmeyen yanıt".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_poll_cases() {
        assert_eq!(parse_poll(r#"{"access_token":"ghu_x","token_type":"bearer"}"#), Poll::Token("ghu_x".into()));
        assert_eq!(parse_poll(r#"{"error":"authorization_pending"}"#), Poll::Pending);
        assert_eq!(parse_poll(r#"{"error":"slow_down","interval":10}"#), Poll::SlowDown);
        assert_eq!(parse_poll(r#"{"error":"expired_token"}"#), Poll::Failed("kodun süresi doldu, yeniden deneyin".into()));
        assert_eq!(parse_poll(r#"{"error":"access_denied"}"#), Poll::Failed("giriş reddedildi".into()));
        assert!(matches!(parse_poll("<html>"), Poll::Failed(_)));
        assert!(matches!(parse_poll(r#"{"access_token":""}"#), Poll::Failed(_)));
    }
}
