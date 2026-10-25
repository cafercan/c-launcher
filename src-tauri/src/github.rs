//! GitHub'dan katalog ve son release bilgisini çeker.
//! Kimlik doğrulamasız API saatte 60 istekle sınırlı; ETag ile önbelleğe alınan yanıtlar
//! (304 Not Modified) bu sınırdan düşmez.

use std::path::Path;

use launcher_core::Catalog;
use serde::{Deserialize, Serialize};

pub const CATALOG_URL: &str = "https://raw.githubusercontent.com/cafercan/c-launcher/main/catalog.json";
pub const LAUNCHER_REPO: &str = "cafercan/c-launcher";
const EMBEDDED_CATALOG: &str = include_str!("../../catalog.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub zipball_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
}

#[derive(Serialize, Deserialize)]
struct Cached {
    etag: Option<String>,
    release: Option<Release>,
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("c-launcher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("http istemcisi")
}

/// Katalog: önce GitHub'daki güncel liste, olmazsa son başarılı kopya, o da yoksa
/// uygulamaya gömülü liste. İkinci değer listenin nereden geldiğini söyler.
pub async fn catalog(http: &reqwest::Client, cache_dir: &Path) -> (Catalog, &'static str) {
    let cache = cache_dir.join("catalog.json");
    if let Ok(resp) = http.get(CATALOG_URL).send().await.and_then(|r| r.error_for_status()) {
        if let Ok(text) = resp.text().await {
            if let Ok(c) = Catalog::parse(&text) {
                let _ = std::fs::write(&cache, &text);
                return (c, "github");
            }
        }
    }
    if let Some(c) = std::fs::read_to_string(&cache).ok().and_then(|t| Catalog::parse(&t).ok()) {
        return (c, "cache");
    }
    (Catalog::parse(EMBEDDED_CATALOG).expect("gömülü katalog geçerli olmalı"), "embedded")
}

/// Deponun son release'i. Hiç release yoksa `Ok(None)`.
/// Ağ hatasında önbellekteki son yanıt kullanılır.
pub async fn latest_release(http: &reqwest::Client, repo: &str, cache_dir: &Path) -> Result<Option<Release>, String> {
    let cache_file = cache_dir.join(format!("release-{}.json", repo.replace('/', "__")));
    let cached: Option<Cached> = std::fs::read_to_string(&cache_file).ok().and_then(|s| serde_json::from_str(&s).ok());
    let mut req = http
        .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
        .header("Accept", "application/vnd.github+json");
    if let Some(etag) = cached.as_ref().and_then(|c| c.etag.clone()) {
        req = req.header("If-None-Match", etag);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return cached.map(|c| c.release).ok_or_else(|| format!("GitHub'a ulaşılamadı: {e}")),
    };
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        if let Some(c) = cached {
            return Ok(c.release);
        }
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        let _ = std::fs::write(&cache_file, serde_json::to_string(&Cached { etag: None, release: None }).unwrap());
        return Ok(None);
    }
    if !status.is_success() {
        // örneğin istek sınırı (403/429): eldeki bilgiyle devam
        return cached.map(|c| c.release).ok_or_else(|| format!("GitHub yanıtı: {status}"));
    }
    let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(String::from);
    let release: Release = resp.json().await.map_err(|e| format!("release okunamadı: {e}"))?;
    let _ = std::fs::write(&cache_file, serde_json::to_string(&Cached { etag, release: Some(release.clone()) }).unwrap());
    Ok(Some(release))
}
