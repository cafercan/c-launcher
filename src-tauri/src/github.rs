//! GitHub'dan katalog ve son release bilgisini çeker.
//! Kimlik doğrulamasız API saatte 60 istekle sınırlı; ETag ile önbelleğe alınan yanıtlar
//! (304 Not Modified) bu sınırdan düşmez.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

use launcher_core::{merge, Brand, Catalog, Group, Part};
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
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
    /// API adresi: girişle okunan depolarda dosya buradan indirilir
    #[serde(default)]
    pub url: String,
}

#[derive(Serialize, Deserialize)]
struct CachedList {
    etag: Option<String>,
    releases: Vec<Release>,
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
        // API, release dosyalarını başka bir sunucuya yönlendirir; reqwest başka sunucuya
        // giden yönlendirmede Authorization başlığını atar, token oraya taşınmaz
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .expect("http istemcisi")
}

/// Token'ı, yalnızca adres GitHub API'siyse isteğe ekler.
pub fn authed(req: reqwest::RequestBuilder, url: &str, token: Option<&str>) -> reqwest::RequestBuilder {
    match token {
        Some(t) if launcher_core::auth::token_allowed(url) => req.bearer_auth(t),
        _ => req,
    }
}

/// Bir istekte token reddedildi (süresi doldu, geri alındı ya da kullanıcı ekipten çıkarıldı).
static UNAUTHORIZED: AtomicBool = AtomicBool::new(false);

/// 401 yalnızca token gönderildiyse girişin geçersiz olduğunu gösterir; 403/404 başka şeydir
/// (istek sınırı, depoya erişim yok).
fn needs_login(status: u16, had_token: bool) -> bool {
    status == 401 && had_token
}

pub fn note_status(status: u16, had_token: bool) {
    if needs_login(status, had_token) {
        UNAUTHORIZED.store(true, Ordering::Relaxed);
    }
}

/// Son çağrıdan bu yana token reddedildi mi? Okuyunca sıfırlanır.
pub fn take_unauthorized() -> bool {
    UNAUTHORIZED.swap(false, Ordering::Relaxed)
}

/// Token'ın sahibinin GitHub kullanıcı adı.
pub async fn user(http: &reqwest::Client, token: &str) -> Result<String, String> {
    let url = "https://api.github.com/user";
    let resp = authed(http.get(url), url, Some(token))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub'a ulaşılamadı: {e}"))?;
    note_status(resp.status().as_u16(), true);
    if !resp.status().is_success() {
        return Err(format!("GitHub yanıtı: {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    v.get("login").and_then(|l| l.as_str()).map(String::from).ok_or_else(|| "kullanıcı adı okunamadı".into())
}

/// Bir katalog kaynağının bu yüklemedeki durumu.
#[derive(Debug, Clone, Serialize)]
pub struct SourceState {
    pub repo: String,
    /// Kaynağın uygulamalarının gösterildiği grup (brand.json'da verildiyse)
    pub group: Option<Group>,
    /// "mirror" | "github" | "cache" | "embedded" | "none" | "login" (giriş yapılmadığı için okunmadı)
    pub from: &'static str,
    pub error: Option<String>,
    /// Kaynağın yerel aynası tanımlı (ofis ağında girişsiz okunur)
    pub has_mirror: bool,
}

/// Yerel aynaya ulaşmak için beklenecek en uzun süre: ofis dışındayken açılışı geciktirmesin.
const MIRROR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

async fn fetch_mirror(http: &reqwest::Client, mirror: &str, file: &str) -> Result<String, String> {
    let resp = http
        .get(format!("{mirror}/{file}"))
        .timeout(MIRROR_TIMEOUT)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("yerel sunucuya ulaşılamadı: {e}"))?;
    resp.text().await.map_err(|e| e.to_string())
}

/// Aynadaki release listesi (`releases.json`, GitHub'ın liste biçiminde; indirme adresleri aynayı gösterir).
pub async fn mirror_releases(http: &reqwest::Client, mirror: &str) -> Result<Vec<Release>, String> {
    let text = fetch_mirror(http, mirror, "releases.json").await?;
    serde_json::from_str(&text).map_err(|e| format!("yerel sunucudaki release listesi okunamadı: {e}"))
}

/// Bütün kaynakların katalogları, birleştirilmiş. Her kaynak için önce GitHub'daki güncel
/// liste, olmazsa o kaynağın son başarılı kopyası, o da yoksa (izin verildiyse) gömülü liste.
pub async fn catalog(http: &reqwest::Client, cache_dir: &Path, token: Option<&str>) -> (Catalog, Vec<SourceState>) {
    let mut parts = Vec::new();
    let mut states = Vec::new();
    for src in &BRAND.sources {
        let mut state = SourceState { repo: src.repo.clone(), group: src.group.clone(), from: "none", error: None, has_mirror: src.mirror.is_some() };
        // brand.json'a gömülü liste: okunacak bir yer yok (geçerliliği derlemede denetlendi)
        if let Some(Ok(c)) = src.inline_catalog() {
            state.from = "brand";
            parts.push(Part { catalog: c, group: src.group.clone(), private: false, mirror: None, hide: src.hide.clone() });
            states.push(state);
            continue;
        }
        let cache = cache_dir.join(format!("catalog-{}.json", src.repo.replace('/', "__")));
        // ofis ağındaysak: yerel aynadan, girişsiz
        if let Some(m) = &src.mirror {
            if let Some((c, text)) = fetch_mirror(http, m, "catalog.json").await.ok().and_then(|t| Catalog::parse(&t).ok().map(|c| (c, t))) {
                let _ = std::fs::write(&cache, text);
                state.from = "mirror";
                parts.push(Part { catalog: c, group: src.group.clone(), private: false, mirror: Some(m.clone()), hide: src.hide.clone() });
                states.push(state);
                continue;
            }
        }
        if src.private && token.is_none() {
            state.from = "login";
            states.push(state);
            continue;
        }
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
            parts.push(Part { catalog: c, group: src.group.clone(), private: src.private, mirror: None, hide: src.hide.clone() });
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
        authed(http.get(&url), &url, token).header("Accept", "application/vnd.github.raw")
    } else {
        http.get(format!("https://raw.githubusercontent.com/{}/{}/{}", src.repo, src.branch, src.path))
    };
    let resp = req.send().await.map_err(|e| format!("GitHub'a ulaşılamadı: {e}"))?;
    note_status(resp.status().as_u16(), src.private && token.is_some());
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
pub async fn latest_release(http: &reqwest::Client, repo: &str, cache_dir: &Path, token: Option<&str>) -> Result<Option<Release>, String> {
    let cache_file = cache_dir.join(format!("release-{}.json", repo.replace('/', "__")));
    let cached: Option<Cached> = std::fs::read_to_string(&cache_file).ok().and_then(|s| serde_json::from_str(&s).ok());
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut req = authed(http.get(&url), &url, token).header("Accept", "application/vnd.github+json");
    if let Some(etag) = cached.as_ref().and_then(|c| c.etag.clone()) {
        req = req.header("If-None-Match", etag);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return cached.map(|c| c.release).ok_or_else(|| format!("GitHub'a ulaşılamadı: {e}")),
    };
    let status = resp.status();
    note_status(status.as_u16(), token.is_some());
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

/// Önekli bir uygulamanın release'i, etiketi sürüm kısmına indirilmiş olarak: sonrası
/// (sürüm karşılaştırma, kurulum) öneki bilmeden çalışır.
pub fn pick_prefixed(list: &[Release], prefix: &str) -> Option<Release> {
    let tags: Vec<(&str, bool)> = list.iter().map(|r| (r.tag_name.as_str(), r.draft || r.prerelease)).collect();
    let tag = launcher_core::select::pick_release(&tags, prefix)?;
    let mut r = list.iter().find(|r| r.tag_name == tag)?.clone();
    r.tag_name = launcher_core::select::strip_tag(tag, Some(prefix)).to_string();
    Some(r)
}

/// Launcher'ın kendi son sürümü; depo başka araçlarla paylaşılıyorsa önekli etiketlerden.
/// Launcher deposunun aynasına bu yüklemede ulaşıldıysa (`mirror`) oradan okunur.
pub async fn launcher_release(http: &reqwest::Client, cache_dir: &Path, token: Option<&str>, mirror: Option<&str>) -> Result<Option<Release>, String> {
    let repo = &BRAND.launcher_repo;
    let prefix = BRAND.launcher_tag_prefix.as_deref();
    if let Some(m) = mirror {
        if let Ok(list) = mirror_releases(http, m).await {
            return Ok(pick_prefixed(&list, prefix.unwrap_or("")));
        }
    }
    match prefix {
        Some(p) => Ok(pick_prefixed(&releases(http, repo, cache_dir, token, &[p]).await?, p)),
        None => latest_release(http, repo, cache_dir, token).await,
    }
}

/// Bu yüklemede aynasına ulaşılan launcher deposu kaynağının ayna adresi.
pub fn launcher_mirror(states: &[SourceState]) -> Option<&'static str> {
    BRAND
        .sources
        .iter()
        .zip(states)
        .find(|(src, st)| src.repo == BRAND.launcher_repo && st.from == "mirror")
        .and_then(|(src, _)| src.mirror.as_deref())
}

const PER_PAGE: usize = 100;
const MAX_PAGES: u32 = 5;

/// Bir sonraki sayfa okunmalı mı: her önek için bir release bulunana kadar, en çok 5 sayfa.
fn need_more(found: &[Release], prefixes: &[&str], page: u32) -> bool {
    page < MAX_PAGES && prefixes.iter().any(|p| pick_prefixed(found, p).is_none())
}

/// Birden çok uygulamanın paylaştığı deponun release listesi. Seyrek release çıkan bir araç
/// diğerlerinin yüzlerce release'inin altında kalabilir: her önek bulunana kadar sayfalar
/// okunur. İlk sayfa ETag ile önbelleğe alınır.
pub async fn releases(http: &reqwest::Client, repo: &str, cache_dir: &Path, token: Option<&str>, prefixes: &[&str]) -> Result<Vec<Release>, String> {
    let cache_file = cache_dir.join(format!("releases-{}.json", repo.replace('/', "__")));
    let cached: Option<CachedList> = std::fs::read_to_string(&cache_file).ok().and_then(|s| serde_json::from_str(&s).ok());
    let mut all = Vec::new();
    let mut page = 1;
    loop {
        let url = format!("https://api.github.com/repos/{repo}/releases?per_page={PER_PAGE}&page={page}");
        let mut req = authed(http.get(&url), &url, token).header("Accept", "application/vnd.github+json");
        let etag = cached.as_ref().filter(|_| page == 1).and_then(|c| c.etag.clone());
        if let Some(e) = &etag {
            req = req.header("If-None-Match", e);
        }
        let fallback = |e: String| match (&cached, page) {
            (Some(c), 1) => Ok(c.releases.clone()),
            (_, 1) => Err(e),
            _ => Ok(Vec::new()), // sonraki sayfalar alınamadıysa eldekiyle devam
        };
        let batch = match req.send().await {
            Err(e) => fallback(format!("GitHub'a ulaşılamadı: {e}"))?,
            Ok(resp) => {
                let status = resp.status();
                note_status(status.as_u16(), token.is_some());
                if let (reqwest::StatusCode::NOT_MODIFIED, 1, Some(c)) = (status, page, &cached) {
                    c.releases.clone()
                } else if !status.is_success() {
                    fallback(format!("GitHub yanıtı: {status}"))?
                } else {
                    let new_etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(String::from);
                    let list: Vec<Release> = resp.json().await.map_err(|e| format!("release listesi okunamadı: {e}"))?;
                    if page == 1 {
                        let _ = std::fs::write(&cache_file, serde_json::to_string(&CachedList { etag: new_etag, releases: list.clone() }).unwrap());
                    }
                    list
                }
            }
        };
        let full = batch.len() >= PER_PAGE;
        all.extend(batch);
        if !full || !need_more(&all, prefixes, page) {
            return Ok(all);
        }
        page += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str) -> Release {
        serde_json::from_value(serde_json::json!({ "tag_name": tag })).unwrap()
    }

    #[test]
    fn pages_until_every_prefix_found() {
        let found = vec![rel("a-v1.0.0"), rel("a-v1.1.0")];
        assert!(need_more(&found, &["a-", "b-"], 1));
        assert!(!need_more(&found, &["a-"], 1));
        assert!(!need_more(&found, &["a-", "b-"], MAX_PAGES));
    }

    /// Tek bir isteğe verilen yanıtla yerel bir HTTP sunucusu; adresini döner.
    fn serve_once(body: &'static str) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn mirror_releases_reads_list_and_fails_fast() {
        let body = r#"[{"tag_name":"tool-v1.2.0","assets":[{"name":"Tool-v1.2.0.exe","size":3,"browser_download_url":"http://192.0.2.10/x/files/tool-v1.2.0/Tool-v1.2.0.exe","url":""}]}]"#;
        let list = mirror_releases(&client(), &serve_once(body)).await.unwrap();
        let r = pick_prefixed(&list, "tool-").unwrap();
        assert_eq!(r.tag_name, "v1.2.0");
        // ayna adresi indirme adresi olarak kalır; token gitmez
        assert!(!launcher_core::auth::token_allowed(&r.assets[0].browser_download_url));
        // kapalı bir port: kısa sürede hata
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let started = std::time::Instant::now();
        assert!(mirror_releases(&client(), &format!("http://{closed}")).await.is_err());
        assert!(started.elapsed() < MIRROR_TIMEOUT + std::time::Duration::from_secs(1));
    }

    #[test]
    fn unprefixed_pick_takes_newest_stable() {
        // aynadaki önek kullanmayan depolar: GitHub'ın "latest" seçimiyle aynı sonuç
        let mut beta = rel("v2.0.0-beta.1");
        beta.prerelease = true;
        let list = vec![rel("v1.0.0"), beta, rel("v1.4.0"), rel("notes")];
        assert_eq!(pick_prefixed(&list, "").unwrap().tag_name, "v1.4.0");
    }

    #[test]
    fn prefixed_release_gets_plain_tag() {
        let list = vec![rel("a-v1.0.0"), rel("b-v2.0.0"), rel("a-v1.2.0")];
        assert_eq!(pick_prefixed(&list, "a-").unwrap().tag_name, "v1.2.0");
        assert!(pick_prefixed(&list, "c-").is_none());
    }

    #[test]
    fn unauthorized_maps_to_login_required() {
        assert!(needs_login(401, true));
        assert!(!needs_login(401, false));
        assert!(!needs_login(403, true));
        assert!(!needs_login(404, true));
    }

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
