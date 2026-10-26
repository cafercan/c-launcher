//! GitHub'dan katalog ve son release bilgisini çeker.
//! Kimlik doğrulamasız API saatte 60 istekle sınırlı; ETag ile önbelleğe alınan yanıtlar
//! (304 Not Modified) bu sınırdan düşmez.

use std::path::Path;
use std::sync::LazyLock;

use launcher_core::{merge, Brand, Catalog, Group};
use serde::{Deserialize, Serialize};

/// Bu derlemenin adı, veri klasörü, katalog kaynakları ve güncelleme deposu
pub static BRAND: LazyLock<Brand> = LazyLock::new(|| Brand::parse(include_str!("../brand.json")).expect("brand.json geçerli olmalı"));
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

/// Bir katalog kaynağının bu yüklemedeki durumu.
#[derive(Debug, Clone, Serialize)]
pub struct SourceState {
    pub repo: String,
    /// Kaynağın uygulamalarının gösterildiği grup (brand.json'da verildiyse)
    pub group: Option<Group>,
    /// "github" | "cache" | "embedded" | "none" | "login" (giriş yapılmadığı için okunmadı)
    pub from: &'static str,
    pub error: Option<String>,
}

/// Bütün kaynakların katalogları, birleştirilmiş. Her kaynak için önce GitHub'daki güncel
/// liste, olmazsa o kaynağın son başarılı kopyası, o da yoksa (izin verildiyse) gömülü liste.
pub async fn catalog(http: &reqwest::Client, cache_dir: &Path, token: Option<&str>) -> (Catalog, Vec<SourceState>) {
    let mut parts = Vec::new();
    let mut states = Vec::new();
    for src in &BRAND.sources {
        let mut state = SourceState { repo: src.repo.clone(), group: src.group.clone(), from: "none", error: None };
        if src.private && token.is_none() {
            state.from = "login";
            states.push(state);
            continue;
        }
        let cache = cache_dir.join(format!("catalog-{}.json", src.repo.replace('/', "__")));
        let fetched = match fetch_catalog(http, src, token).await {
            Ok(t) => Some(t),
            Err(e) => {
                state.error = Some(e);
                None
            }
        };
        // 0.1.x tek kaynağın kopyasını catalog.json adıyla tutuyordu
        let cached = std::fs::read_to_string(&cache)
            .ok()
            .or_else(|| src.embedded.then(|| std::fs::read_to_string(cache_dir.join("catalog.json")).ok()).flatten());
        let embedded = src.embedded.then_some(EMBEDDED_CATALOG);
        let (catalog, from) = pick_source(fetched.as_deref(), cached.as_deref(), embedded);
        if from == "github" {
            let _ = std::fs::write(&cache, fetched.unwrap_or_default());
        }
        state.from = from;
        if let Some(c) = catalog {
            parts.push((c, src.group.clone(), src.private));
        }
        states.push(state);
    }
    let merged = merge(parts);
    match merged.validate() {
        Ok(()) => (merged, states),
        // birleşince bozulduysa (ör. bilinmeyen grup) güvenli tarafta kal
        Err(e) => {
            for s in &mut states {
                s.error.get_or_insert_with(|| e.clone());
            }
            (Catalog { schema: launcher_core::catalog::SCHEMA, groups: Vec::new(), apps: Vec::new() }, states)
        }
    }
}

async fn fetch_catalog(http: &reqwest::Client, src: &launcher_core::Source, token: Option<&str>) -> Result<String, String> {
    let req = if src.private {
        let url = format!("https://api.github.com/repos/{}/contents/{}?ref={}", src.repo, src.path, src.branch);
        let mut req = http.get(url).header("Accept", "application/vnd.github.raw");
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        req
    } else {
        http.get(format!("https://raw.githubusercontent.com/{}/{}/{}", src.repo, src.branch, src.path))
    };
    let resp = req.send().await.map_err(|e| format!("GitHub'a ulaşılamadı: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("katalog alınamadı: {}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

/// Hangi kopyanın kullanılacağı: geçerli olan ilk aday. Geçersiz bir GitHub yanıtı
/// (yarım dosya, hatalı düzenleme) önbelleğin yerine geçmez.
fn pick_source(fetched: Option<&str>, cached: Option<&str>, embedded: Option<&str>) -> (Option<Catalog>, &'static str) {
    [(fetched, "github"), (cached, "cache"), (embedded, "embedded")]
        .into_iter()
        .find_map(|(text, from)| text.and_then(|t| Catalog::parse(t).ok()).map(|c| (Some(c), from)))
        .unwrap_or((None, "none"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_falls_back_per_source() {
        let good = r#"{"schema":1,"groups":[],"apps":[]}"#;
        assert_eq!(pick_source(Some("{bozuk"), Some(good), None).1, "cache");
        assert_eq!(pick_source(Some(good), Some(good), Some(good)).1, "github");
        assert_eq!(pick_source(None, None, Some(good)).1, "embedded");
        let (c, from) = pick_source(None, None, None);
        assert!(c.is_none());
        assert_eq!(from, "none");
    }
}
