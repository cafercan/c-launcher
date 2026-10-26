//! GitHub cihaz koduyla giriş: launcher bir kod alır, kullanıcı github.com/login/device
//! sayfasında bu kodu girip onaylar, launcher onay gelene kadar token ister.
//! https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app#using-the-device-flow-to-generate-a-user-access-token

use std::time::{Duration, Instant};

use launcher_core::device::{parse_poll, Poll};
use serde::{Deserialize, Serialize};

/// Kullanıcıya gösterilecek kısım
#[derive(Clone, Serialize)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
}

/// Token isteği için saklanan kısım
pub struct Pending {
    device_code: String,
    interval: u64,
    expires_at: Instant,
}

#[derive(Deserialize)]
struct CodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    #[serde(default = "five")]
    interval: u64,
}

fn five() -> u64 {
    5
}

async fn post(http: &reqwest::Client, url: &str, body: String) -> Result<String, String> {
    http.post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("GitHub'a ulaşılamadı: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())
}

/// Client ID ve cihaz kodu yalnızca harf, rakam, nokta, tire ve alt çizgiden oluşur;
/// başka bir karakter gelirse form gövdesine konmaz.
fn form_safe(s: &str) -> Result<&str, String> {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        Ok(s)
    } else {
        Err("geçersiz giriş bilgisi".into())
    }
}

pub async fn start(http: &reqwest::Client, client_id: &str) -> Result<(DeviceCode, Pending), String> {
    let text = post(http, "https://github.com/login/device/code", format!("client_id={}", form_safe(client_id)?)).await?;
    let r: CodeResponse = serde_json::from_str(&text).map_err(|_| "GitHub giriş kodu vermedi".to_string())?;
    let code = DeviceCode { user_code: r.user_code, verification_uri: r.verification_uri, expires_in: r.expires_in };
    let pending = Pending { device_code: r.device_code, interval: r.interval.max(1), expires_at: Instant::now() + Duration::from_secs(r.expires_in) };
    Ok((code, pending))
}

/// Kullanıcı onaylayana, reddedene ya da kodun süresi dolana kadar bekler; token döner.
pub async fn wait(http: &reqwest::Client, client_id: &str, mut p: Pending) -> Result<String, String> {
    let body = format!(
        "client_id={}&device_code={}&grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code",
        form_safe(client_id)?,
        form_safe(&p.device_code)?
    );
    loop {
        tokio::time::sleep(Duration::from_secs(p.interval)).await;
        if Instant::now() >= p.expires_at {
            return Err("kodun süresi doldu, yeniden deneyin".into());
        }
        match parse_poll(&post(http, "https://github.com/login/oauth/access_token", body.clone()).await?) {
            Poll::Token(t) => return Ok(t),
            Poll::Pending => {}
            Poll::SlowDown => p.interval += 5,
            Poll::Failed(e) => return Err(e),
        }
    }
}
