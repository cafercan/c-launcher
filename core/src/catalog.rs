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
    /// Birden çok uygulama aynı depoda release yayımlıyorsa bu uygulamanın etiket öneki
    /// ("alpha-" -> "alpha-v0.1.5")
    #[serde(default)]
    pub tag_prefix: Option<String>,
    /// Girişle okunan bir katalogdan geldi: release'leri de girişle okunur
    #[serde(skip)]
    pub private: bool,
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
    /// Windows: kurulumsuz çalışan tek .exe
    Exe,
    /// Windows: kurulumsuz uygulama klasörü (.zip, kökünde tek klasör)
    Zip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformSpec {
    pub method: Method,
    /// Release dosyası için `*` içerebilen ad kalıbı
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub inno_app_id: Option<String>,
    /// macOS: /Applications altındaki paket adı; `exe` yöntemi: diskteki dosya adı
    #[serde(default)]
    pub app: Option<String>,
    /// Linux: dpkg paket adı
    #[serde(default)]
    pub package: Option<String>,
    /// Linux: uygulamayı başlatan komut; `zip` yöntemi: arşivdeki tek klasörün içinde çalıştırılacak .exe
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
    /// Betik yöntemi: bunlardan biri varsa kurulu sayılır. `~/` ev dizini, `{documents}/`
    /// Belgeler klasörüdür. `detect_path`'e ek olarak ayrı bir alan: eski launcher'lar
    /// tanımadığı alanı yok sayar, tanımadığı yer tutucuyu ise yanlış yorumlardı.
    #[serde(default)]
    pub detect_paths: Vec<String>,
    /// Linux: kullanıcı bir kurulum klasörü seçtiyse .deb yerine kullanılacak taşınabilir paket
    #[serde(default)]
    pub portable: Option<Portable>,
}

/// Seçilen klasöre açılan `.tar.gz` paketi.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portable {
    /// Release dosyası kalıbı, örneğin "kitaplik-*-linux-portable.tar.gz"
    pub asset: String,
    /// Arşivin içinde uygulamayı başlatan dosya
    pub run: String,
    /// Arşivin içinde menü simgesi
    #[serde(default)]
    pub icon: Option<String>,
    /// Pencerenin sınıf adı: menüdeki kısayolla eşleşsin diye
    #[serde(default)]
    pub wm_class: Option<String>,
}

impl App {
    /// Bu sistem için kurulum tarifi; yoksa uygulama bu sistemde sunulmuyor demektir.
    pub fn spec_for(&self, os: Os) -> Option<&PlatformSpec> {
        self.platforms.get(os.key()).or_else(|| self.platforms.get("any"))
    }
}

impl PlatformSpec {
    /// Betik yöntemi: kurulu olduğunu gösterebilecek bütün yollar.
    pub fn detect_candidates(&self) -> impl Iterator<Item = &str> {
        self.detect_path.iter().chain(&self.detect_paths).map(String::as_str)
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
            if app.tag_prefix.as_deref().is_some_and(|p| !valid_prefix(p)) {
                return Err(format!("{}: geçersiz etiket öneki (küçük harf, rakam, tire; sonu tire)", app.id));
            }
            let shared: Vec<&App> = self.apps.iter().filter(|a| a.repo == app.repo).collect();
            if shared.len() > 1 {
                let prefix = app.tag_prefix.as_deref().ok_or_else(|| format!("{}: {} deposunu paylaşan uygulamalar etiket öneki taşımalı", app.id, app.repo))?;
                if shared.iter().filter(|a| a.tag_prefix.as_deref() == Some(prefix)).count() > 1 {
                    return Err(format!("{}: {prefix} öneki aynı depoda iki kez kullanılmış", app.id));
                }
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
                    Method::Exe | Method::Zip => {
                        if os != "windows" {
                            let m = if spec.method == Method::Exe { "exe" } else { "zip" };
                            return Err(format!("{where_}: {m} yöntemi yalnızca windows için"));
                        }
                        need(&spec.asset, "asset")?;
                        let exe_ok = |p: &str| crate::paths::is_plain_relative(p) && p.to_ascii_lowercase().ends_with(".exe");
                        if spec.method == Method::Zip && !spec.run.as_deref().is_some_and(exe_ok) {
                            return Err(format!("{where_}: run, arşivdeki klasörün içinde bir .exe yolu olmalı"));
                        }
                        // dosya adı tek parça olmalı: klasör adı ya da üst klasör olamaz
                        if spec.method == Method::Exe && spec.app.as_deref().is_some_and(|a| !exe_ok(a) || a.contains('/')) {
                            return Err(format!("{where_}: app bir .exe dosya adı olmalı"));
                        }
                    }
                }
                if let Some(p) = &spec.portable {
                    if p.asset.is_empty() || !crate::paths::is_plain_relative(&p.run) || p.icon.as_deref().is_some_and(|i| !crate::paths::is_plain_relative(i)) {
                        return Err(format!("{where_}: taşınabilir paket tarifi geçersiz"));
                    }
                }
                if spec.detect_candidates().any(str::is_empty) {
                    return Err(format!("{where_}: boş algılama yolu"));
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

/// Birden çok kaynağın kataloglarını sırayla birleştirir. Aynı kimlik iki kaynakta varsa
/// öndeki kazanır. Grup verilmiş bir kaynağın bütün uygulamaları o gruba taşınır.
pub fn merge(parts: Vec<(Catalog, Option<Group>, bool)>) -> Catalog {
    let mut out = Catalog { schema: SCHEMA, groups: Vec::new(), apps: Vec::new() };
    for (catalog, group, private) in parts {
        let groups = match &group {
            Some(g) => vec![g.clone()],
            None => catalog.groups,
        };
        for g in groups {
            if !out.groups.iter().any(|x| x.id == g.id) {
                out.groups.push(g);
            }
        }
        for mut app in catalog.apps {
            if out.apps.iter().any(|a| a.id == app.id) {
                continue;
            }
            if let Some(g) = &group {
                app.group = g.id.clone();
            }
            app.private = private;
            out.apps.push(app);
        }
    }
    out
}

/// Etiket öneki: küçük harf, rakam ve tire, sonu tire ("alpha-").
pub(crate) fn valid_prefix(p: &str) -> bool {
    p.len() > 1 && p.ends_with('-') && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Uygulama kimliği klasör adı olarak da kullanılır: yalnızca küçük harf, rakam ve tire.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub(crate) fn valid_repo(repo: &str) -> bool {
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
        assert_eq!(md.spec_for(Os::Macos).unwrap().method, Method::Dmg);
    }

    #[test]
    fn script_tools_are_detectable_everywhere() {
        // elle kurulmuş bir terminal aracı da "kurulu" görünmeli: her sistemde bir algılama yolu gerekir
        let c = Catalog::parse(REAL).unwrap();
        for app in c.apps.iter() {
            for (os, spec) in &app.platforms {
                if spec.method == Method::Script {
                    assert!(spec.detect_candidates().next().is_some(), "{} / {os}: algılama yolu yok", app.id);
                }
            }
        }
    }

    #[test]
    fn detect_candidates_merge_both_fields() {
        let c = with(r#"{"id":"a","repo":"x/y","name":"A","group":"desktop","platforms":{"windows":{"method":"script","install":"i.ps1","detect_path":"~/a","detect_paths":["{documents}/b"]}}}"#).unwrap();
        let spec = c.apps[0].spec_for(Os::Windows).unwrap();
        assert_eq!(spec.detect_candidates().collect::<Vec<_>>(), ["~/a", "{documents}/b"]);
        assert!(with(r#"{"id":"a","repo":"x/y","name":"A","group":"desktop","platforms":{"windows":{"method":"script","install":"i.ps1","detect_paths":[""]}}}"#).is_err());
    }

    fn with(app_json: &str) -> Result<Catalog, String> {
        Catalog::parse(&format!(
            r#"{{"schema":1,"groups":[{{"id":"desktop","title":"D"}}],"apps":[{app_json}]}}"#
        ))
    }

    fn cat(groups: &[&str], apps: &[(&str, &str)]) -> Catalog {
        let groups: Vec<String> = groups.iter().map(|g| format!(r#"{{"id":"{g}","title":"{g}"}}"#)).collect();
        let apps: Vec<String> = apps.iter().map(|(id, g)| format!(r#"{{"id":"{id}","repo":"example-org/{id}","name":"{id}","group":"{g}","platforms":{{}}}}"#)).collect();
        Catalog::parse(&format!(r#"{{"schema":1,"groups":[{}],"apps":[{}]}}"#, groups.join(","), apps.join(","))).unwrap()
    }

    #[test]
    fn merge_moves_apps_into_override_group() {
        let a = cat(&["tools"], &[("t", "tools")]);
        let b = cat(&["desktop", "chrome"], &[("x", "desktop"), ("y", "chrome")]);
        let shared = Group { id: "shared".into(), title: "Ortak uygulamalar".into() };
        let m = merge(vec![(a, None, true), (b, Some(shared), false)]);
        let ids: Vec<&str> = m.groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, ["tools", "shared"]);
        assert!(m.apps.iter().filter(|a| a.id != "t").all(|a| a.group == "shared" && !a.private));
        assert!(m.apps.iter().find(|a| a.id == "t").unwrap().private);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn merge_drops_duplicate_ids() {
        let a = cat(&["desktop"], &[("x", "desktop")]);
        let mut b = cat(&["desktop"], &[("x", "desktop")]);
        b.apps[0].name = "B".into();
        let m = merge(vec![(a, None, false), (b, None, false)]);
        assert_eq!(m.apps.len(), 1);
        assert_eq!(m.apps[0].name, "x");
    }

    #[test]
    fn merge_keeps_group_order() {
        let a = cat(&["desktop", "chrome"], &[]);
        let b = cat(&["terminal", "desktop"], &[]);
        let m = merge(vec![(a, None, false), (b, None, false)]);
        let ids: Vec<&str> = m.groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, ["desktop", "chrome", "terminal"]);
    }

    #[test]
    fn shared_repo_needs_distinct_prefixes() {
        let two = |a: &str, b: &str| {
            let app = |id: &str, prefix: &str| {
                let p = if prefix.is_empty() { String::new() } else { format!(r#","tag_prefix":"{prefix}""#) };
                format!(r#"{{"id":"{id}","repo":"example-org/releases","name":"{id}","group":"desktop","platforms":{{}}{p}}}"#)
            };
            Catalog::parse(&format!(r#"{{"schema":1,"groups":[{{"id":"desktop","title":"D"}}],"apps":[{},{}]}}"#, app("a", a), app("b", b)))
        };
        assert!(two("tool-", "").is_err());
        assert!(two("tool-", "tool-").is_err());
        assert!(two("Tool-", "other-").is_err());
        assert!(two("tool", "other-").is_err());
        assert!(two("tool-", "other-").is_ok());
    }

    #[test]
    fn portable_methods_validate() {
        let one = |os: &str, spec: &str| with(&format!(r#"{{"id":"a","repo":"x/y","name":"A","group":"desktop","platforms":{{"{os}":{spec}}}}}"#));
        assert!(one("windows", r#"{"method":"exe"}"#).is_err());
        assert!(one("windows", r#"{"method":"exe","asset":"Tool-v*.exe"}"#).is_ok());
        assert!(one("windows", r#"{"method":"exe","asset":"Tool-v*.exe","app":"Tool.exe"}"#).is_ok());
        assert!(one("windows", r#"{"method":"exe","asset":"Tool-v*.exe","app":"../Tool.exe"}"#).is_err());
        assert!(one("linux", r#"{"method":"exe","asset":"Tool-v*.exe"}"#).is_err());
        assert!(one("any", r#"{"method":"zip","asset":"t.zip","run":"tool.exe"}"#).is_err());
        assert!(one("windows", r#"{"method":"zip","asset":"t.zip"}"#).is_err());
        assert!(one("windows", r#"{"method":"zip","asset":"t.zip","run":"../tool.exe"}"#).is_err());
        assert!(one("windows", r#"{"method":"zip","asset":"t.zip","run":"tool.txt"}"#).is_err());
        assert!(one("windows", r#"{"method":"zip","asset":"t.zip","run":"tool.exe"}"#).is_ok());
        assert!(one("windows", r#"{"method":"zip","asset":"t.zip","run":"bin/tool.exe"}"#).is_ok());
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
        let deb = r#""method":"deb","asset":"a.deb","package":"a""#;
        assert!(with(&format!(r#"{{"id":"a","repo":"x/y","name":"A","group":"desktop","platforms":{{"linux":{{{deb},"portable":{{"asset":"a.tar.gz","run":"a/run"}}}}}}}}"#)).is_ok());
        assert!(with(&format!(r#"{{"id":"a","repo":"x/y","name":"A","group":"desktop","platforms":{{"linux":{{{deb},"portable":{{"asset":"a.tar.gz","run":"/bin/sh"}}}}}}}}"#)).is_err());
    }
}
