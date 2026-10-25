//! `catalog.json`: launcher'da görünecek uygulamaların listesi.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Os;

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    pub groups: Vec<Group>,
    pub apps: Vec<App>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub name: String,
    /// "sahip/depo"
    pub repo: String,
    pub group: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// Terminal araçları için kısa kullanım bilgisi
    #[serde(default)]
    pub usage: Option<String>,
    /// Anahtarlar: "windows", "macos", "linux" ya da her sistem için "any"
    pub platforms: BTreeMap<String, PlatformSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    /// Windows: Inno Setup kurulum .exe'si
    Inno,
    /// macOS: .app içeren .dmg
    Dmg,
    /// Linux: .deb paketi
    Deb,
    /// Kaynak koddaki kurulum betiği (terminal araçları)
    Script,
    /// Paketlenmemiş Chrome eklentisi (.zip)
    Chrome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformSpec {
    pub method: Method,
    /// Release dosyası için `*` içerebilen ad kalıbı
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub inno_app_id: Option<String>,
    /// macOS: /Applications altındaki paket adı
    #[serde(default)]
    pub app: Option<String>,
    /// Linux: dpkg paket adı
    #[serde(default)]
    pub package: Option<String>,
    /// Linux: uygulamayı başlatan komut
    #[serde(default)]
    pub run: Option<String>,
    /// Betik yöntemi: depo kökündeki kurulum/kaldırma betikleri
    #[serde(default)]
    pub install: Option<String>,
    #[serde(default)]
    pub uninstall: Option<String>,
    /// Betik yöntemi: varsa kurulu olduğunu gösteren dosya (~ ev dizinidir)
    #[serde(default)]
    pub detect_path: Option<String>,
}

impl App {
    /// Bu sistem için kurulum tarifi; yoksa uygulama bu sistemde sunulmuyor demektir.
    pub fn spec_for(&self, os: Os) -> Option<&PlatformSpec> {
        self.platforms.get(os.key()).or_else(|| self.platforms.get("any"))
    }
}

impl Catalog {
    pub fn parse(json: &str) -> Result<Catalog, String> {
        let catalog: Catalog = serde_json::from_str(json).map_err(|e| format!("katalog okunamadı: {e}"))?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Kurulumu güvensiz ya da imkânsız kılacak hataları yakalar.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!("desteklenmeyen katalog sürümü: {}", self.schema));
        }
        let mut seen = std::collections::HashSet::new();
        for app in &self.apps {
            if !seen.insert(app.id.as_str()) {
                return Err(format!("aynı kimlik iki kez kullanılmış: {}", app.id));
            }
            if !valid_id(&app.id) {
                return Err(format!("geçersiz uygulama kimliği: {}", app.id));
            }
            if !valid_repo(&app.repo) {
                return Err(format!("{}: geçersiz depo adı {}", app.id, app.repo));
            }
            if !self.groups.iter().any(|g| g.id == app.group) {
                return Err(format!("{}: bilinmeyen grup {}", app.id, app.group));
            }
            for (os, spec) in &app.platforms {
                let where_ = format!("{} / {}", app.id, os);
                let need = |field: &Option<String>, name: &str| {
                    if field.as_deref().is_none_or(str::is_empty) {
                        Err(format!("{where_}: {name} gerekli"))
                    } else {
                        Ok(())
                    }
                };
                match spec.method {
                    Method::Inno => {
                        need(&spec.asset, "asset")?;
                        need(&spec.inno_app_id, "inno_app_id")?;
                    }
                    Method::Dmg => {
                        need(&spec.asset, "asset")?;
                        need(&spec.app, "app")?;
                    }
                    Method::Deb => {
                        need(&spec.asset, "asset")?;
                        need(&spec.package, "package")?;
                    }
                    Method::Script => need(&spec.install, "install")?,
                    Method::Chrome => need(&spec.asset, "asset")?,
                }
                for script in [&spec.install, &spec.uninstall].into_iter().flatten() {
                    if !crate::paths::is_plain_relative(script) {
                        return Err(format!("{where_}: betik yolu depo içinde kalmalı: {script}"));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Uygulama kimliği klasör adı olarak da kullanılır: yalnızca küçük harf, rakam ve tire.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_repo(repo: &str) -> bool {
    let ok = |s: &str| !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    matches!(repo.split_once('/'), Some((owner, name)) if ok(owner) && ok(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = include_str!("../../catalog.json");

    #[test]
    fn real_catalog_is_valid() {
        let c = Catalog::parse(REAL).unwrap();
        assert_eq!(c.groups.len(), 3);
        assert!(c.apps.iter().any(|a| a.id == "kitaplik"));
    }

    #[test]
    fn spec_falls_back_to_any() {
        let c = Catalog::parse(REAL).unwrap();
        let kedy = c.apps.iter().find(|a| a.id == "kedy").unwrap();
        assert_eq!(kedy.spec_for(Os::Windows).unwrap().method, Method::Chrome);
        let md = c.apps.iter().find(|a| a.id == "mdviewer").unwrap();
        assert!(md.spec_for(Os::Macos).is_none(), "MDViewer'ın Mac sürümü yok");
    }

    fn with(app_json: &str) -> Result<Catalog, String> {
        Catalog::parse(&format!(
            r#"{{"schema":1,"groups":[{{"id":"desktop","title":"D"}}],"apps":[{app_json}]}}"#
        ))
    }

    #[test]
    fn rejects_bad_input() {
        let base = r#""name":"A","group":"desktop","platforms":{}"#;
        assert!(with(&format!(r#"{{"id":"a","repo":"x/y",{base}}}"#)).is_ok());
        assert!(with(&format!(r#"{{"id":"../a","repo":"x/y",{base}}}"#)).is_err());
        assert!(with(&format!(r#"{{"id":"a","repo":"x/../y",{base}}}"#)).is_err());
        assert!(with(r#"{"id":"a","repo":"x/y","name":"A","group":"nope","platforms":{}}"#).is_err());
        assert!(with(r#"{"id":"a","repo":"x/y","name":"A","group":"desktop","platforms":{"linux":{"method":"deb","asset":"a.deb"}}}"#).is_err());
        assert!(with(r#"{"id":"a","repo":"x/y","name":"A","group":"desktop","platforms":{"linux":{"method":"script","install":"../evil.sh"}}}"#).is_err());
    }
}
