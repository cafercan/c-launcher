//! `brand.json`: derlemeye gömülen ayarlar. Aynı kaynaktan farklı adla, farklı kataloglarla
//! ve isteğe bağlı GitHub girişiyle ayrı bir launcher derlenebilsin diye.

use serde::{Deserialize, Serialize};

use crate::catalog::{valid_prefix, valid_repo, Group};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brand {
    /// Pencerede ve menülerde görünen ad
    pub name: String,
    /// Kullanıcı verisinin tutulduğu klasör adı (data_local_dir altında)
    pub data_dir: String,
    /// Launcher'ın kendi güncellemelerinin okunduğu depo
    pub launcher_repo: String,
    /// Depo başka araçların release'lerini de taşıyorsa launcher'ın etiket öneki
    #[serde(default)]
    pub launcher_tag_prefix: Option<String>,
    /// Alt çubuktaki GITHUB bağlantısı
    pub home_url: String,
    /// Katalogların okunduğu depolar; listeler sırayla birleştirilir
    pub sources: Vec<Source>,
    #[serde(default)]
    pub login: Option<Login>,
    /// Gruplar sekme olarak gösterilsin
    #[serde(default)]
    pub group_tabs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// "sahip/depo"
    pub repo: String,
    #[serde(default = "default_path")]
    pub path: String,
    #[serde(default = "default_branch")]
    pub branch: String,
    /// Okumak için GitHub girişi gerekir
    #[serde(default)]
    pub private: bool,
    /// Ulaşılamazsa uygulamaya gömülü catalog.json kullanılır
    #[serde(default)]
    pub embedded: bool,
    /// Yerel ağdaki ayna (ör. "http://192.0.2.10/araclar"): ulaşılabiliyorsa katalog ve
    /// release'ler girişsiz buradan okunur (`catalog.json`, `releases.json`)
    #[serde(default)]
    pub mirror: Option<String>,
    /// Bu kaynağın bütün uygulamaları bu grupta gösterilir
    #[serde(default)]
    pub group: Option<Group>,
    /// Bu kaynağın kataloğunda olup bu launcher'da gösterilmeyecek uygulama kimlikleri
    #[serde(default)]
    pub hide: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Login {
    /// Cihaz koduyla giriş açık bir GitHub App'in Client ID'si
    pub client_id: String,
}

fn default_path() -> String {
    "catalog.json".into()
}

fn default_branch() -> String {
    "main".into()
}

impl Brand {
    pub fn parse(json: &str) -> Result<Brand, String> {
        let brand: Brand = serde_json::from_str(json).map_err(|e| format!("brand.json okunamadı: {e}"))?;
        brand.validate()?;
        Ok(brand)
    }

    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("brand.json: name boş".into());
        }
        let d = &self.data_dir;
        if d.is_empty() || d == "." || d == ".." || d.contains(['/', '\\', ':']) {
            return Err(format!("brand.json: geçersiz data_dir {d}"));
        }
        if !valid_repo(&self.launcher_repo) {
            return Err(format!("brand.json: geçersiz launcher_repo {}", self.launcher_repo));
        }
        // alt çubuktaki bağlantı yalnızca github.com sayfalarını açabilir
        if !self.home_url.starts_with("https://github.com/") {
            return Err("brand.json: home_url bir https://github.com/ adresi olmalı".into());
        }
        if self.launcher_tag_prefix.as_deref().is_some_and(|p| !valid_prefix(p)) {
            return Err("brand.json: geçersiz launcher_tag_prefix".into());
        }
        if self.sources.is_empty() {
            return Err("brand.json: en az bir katalog kaynağı gerekli".into());
        }
        if self.login.as_ref().is_some_and(|l| l.client_id.trim().is_empty()) {
            return Err("brand.json: login.client_id boş".into());
        }
        for s in &self.sources {
            if !valid_repo(&s.repo) || !crate::paths::is_plain_relative(&s.path) || s.branch.is_empty() {
                return Err(format!("brand.json: geçersiz katalog kaynağı {}", s.repo));
            }
            if s.mirror.as_deref().is_some_and(|m| !valid_mirror(m)) {
                return Err(format!("brand.json: {}: ayna adresi http:// ya da https:// ile başlamalı, / ile bitmemeli", s.repo));
            }
            if s.private && self.login.is_none() {
                return Err(format!("brand.json: {}: özel katalog giriş gerektirir", s.repo));
            }
        }
        Ok(())
    }
}

fn valid_mirror(m: &str) -> bool {
    let rest = m.strip_prefix("http://").or_else(|| m.strip_prefix("https://"));
    rest.is_some_and(|r| !r.is_empty() && !r.ends_with('/') && !r.contains(|c: char| c.is_whitespace() || "\"'<>\\?#".contains(c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_brand_matches_today() {
        let b = Brand::parse(include_str!("../../src-tauri/brand.json")).unwrap();
        assert_eq!(b.name, "C Launcher");
        assert_eq!(b.data_dir, "c-launcher");
        assert_eq!(b.launcher_repo, "cafercan/c-launcher");
        assert_eq!(b.sources.len(), 1);
        let s = &b.sources[0];
        assert_eq!((s.repo.as_str(), s.path.as_str(), s.branch.as_str()), ("cafercan/c-launcher", "catalog.json", "main"));
        assert!(s.embedded && !s.private && s.group.is_none());
        assert!(b.login.is_none());
        assert!(!b.group_tabs);
        assert!(b.launcher_tag_prefix.is_none());
    }

    fn with(edit: &str) -> Result<Brand, String> {
        let base = r#"{"name":"X","data_dir":"x","launcher_repo":"example-org/x","home_url":"https://github.com/example-org","sources":[{"repo":"example-org/x"}]}"#;
        let mut v: serde_json::Value = serde_json::from_str(base).unwrap();
        let patch: serde_json::Value = serde_json::from_str(edit).unwrap();
        for (k, val) in patch.as_object().unwrap() {
            v[k] = val.clone();
        }
        Brand::parse(&v.to_string())
    }

    #[test]
    fn rejects_bad_brand() {
        assert!(with("{}").is_ok());
        assert!(with(r#"{"sources":[]}"#).is_err());
        assert!(with(r#"{"data_dir":"a/b"}"#).is_err());
        assert!(with(r#"{"data_dir":".."}"#).is_err());
        assert!(with(r#"{"launcher_repo":"nope"}"#).is_err());
        assert!(with(r#"{"home_url":"https://example.org"}"#).is_err());
        assert!(with(r#"{"launcher_tag_prefix":"launcher"}"#).is_err());
        assert!(with(r#"{"launcher_tag_prefix":"launcher-"}"#).is_ok());
        assert!(with(r#"{"login":{"client_id":""}}"#).is_err());
        let err = with(r#"{"sources":[{"repo":"example-org/tools","private":true}]}"#).unwrap_err();
        assert!(err.contains("özel katalog giriş gerektirir"), "{err}");
        assert!(with(r#"{"login":{"client_id":"Iv1.abc"},"sources":[{"repo":"example-org/tools","private":true}]}"#).is_ok());
        assert!(with(r#"{"sources":[{"repo":"example-org/x","path":"../catalog.json"}]}"#).is_err());
        assert!(with(r#"{"sources":[{"repo":"example-org/x","mirror":"http://192.0.2.10/tools"}]}"#).is_ok());
        assert!(with(r#"{"sources":[{"repo":"example-org/x","mirror":"https://mirror.example.org"}]}"#).is_ok());
        assert!(with(r#"{"sources":[{"repo":"example-org/x","mirror":"http://192.0.2.10/tools/"}]}"#).is_err());
        assert!(with(r#"{"sources":[{"repo":"example-org/x","mirror":"ftp://192.0.2.10"}]}"#).is_err());
        assert!(with(r#"{"sources":[{"repo":"example-org/x","mirror":"http://"}]}"#).is_err());
    }
}
