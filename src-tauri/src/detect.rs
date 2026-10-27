//! Bir uygulamanın bu bilgisayarda kurulu olup olmadığını ve sürümünü bulur.
//! Elle kurulmuş uygulamalar da bulunur: launcher'ın kendi kaydına güvenilmez,
//! mümkün olan her yerde sistemin kendisine bakılır.

use std::path::{Path, PathBuf};

use launcher_core::{App, Method, PlatformSpec};

use crate::store::Store;

#[derive(Debug, Clone, Default)]
pub struct Installed {
    /// None: kurulu ama sürümü okunamadı
    pub version: Option<String>,
    /// Uygulamayı açmak için çalıştırılacak dosya ya da paket yolu
    pub target: Option<PathBuf>,
    /// Windows: sessiz kaldırma komutu
    pub uninstall: Option<String>,
    /// Linux: .deb yerine seçilen klasöre taşınabilir paket olarak kuruldu
    pub portable: bool,
}

pub fn installed(id: &str, spec: &PlatformSpec, store: &Store) -> Option<Installed> {
    match spec.method {
        Method::Inno => inno(spec.inno_app_id.as_deref()?, spec.app.as_deref()),
        Method::Dmg => {
            let app = spec.app.as_deref()?;
            // önce launcher'ın seçilen klasöre kurduğu kopya
            let custom = store.record(id).and_then(|r| r.location).map(|l| PathBuf::from(l).join(app)).filter(|p| p.exists());
            match custom {
                Some(path) => Some(Installed { version: bundle_version(&path), target: Some(path), ..Default::default() }),
                None => dmg(app),
            }
        }
        Method::Deb => deb(spec.package.as_deref()?).or_else(|| portable(id, spec, store)),
        Method::Script => {
            let record = store.record(id);
            // tanımlı bir algılama yolu yoksa yalnızca launcher'ın kaydına bakılır
            let (home, documents) = (dirs::home_dir(), dirs::document_dir());
            let mut candidates = spec.detect_candidates().peekable();
            let on_disk = candidates.peek().is_some().then(|| {
                candidates
                    .filter_map(|p| launcher_core::paths::expand_known(p, home.as_deref(), documents.as_deref()))
                    .any(|p| p.exists())
            });
            match (record, on_disk) {
                // kayıt var ama araç diskten silinmiş: kurulu değil
                (Some(_), Some(false)) => None,
                (Some(r), _) => Some(Installed { version: Some(r.version), ..Default::default() }),
                (None, Some(true)) => Some(Installed::default()),
                _ => None,
            }
        }
        Method::Exe | Method::Zip => {
            let record = store.record(id);
            let dir = portable_location(record.as_ref().and_then(|r| r.location.as_deref()).map(Path::new), &portable_default_dir(store, id), spec, id, |p| p.exists())?;
            let exe = portable_exe(&dir, spec, id);
            Some(Installed { version: record.map(|r| r.version), target: Some(exe), ..Default::default() })
        }
        Method::Chrome => {
            let dir = extension_dir(store, id);
            let manifest = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
            let version = serde_json::from_str::<serde_json::Value>(&manifest)
                .ok()
                .and_then(|m| m.get("version").and_then(|v| v.as_str()).map(String::from));
            Some(Installed { version, target: Some(dir), ..Default::default() })
        }
    }
}

/// Linux: seçilen klasöre açılmış taşınabilir paket.
fn portable(id: &str, spec: &PlatformSpec, store: &Store) -> Option<Installed> {
    let p = spec.portable.as_ref()?;
    let record = store.record(id)?;
    let run = PathBuf::from(record.location?).join(&p.run);
    run.exists().then(|| Installed { version: Some(record.version), target: Some(run), portable: true, ..Default::default() })
}

/// Windows kurulumsuz uygulamanın varsayılan klasörü: seçilen kurulum klasörü, yoksa
/// %LOCALAPPDATA%\Programs\<launcher> altında uygulamanın kimliği.
pub fn portable_default_dir(store: &Store, id: &str) -> PathBuf {
    let root = store.install_root().unwrap_or_else(|| {
        dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("Programs").join(&crate::github::BRAND.data_dir)
    });
    root.join(id)
}

/// Kurulumsuz uygulamanın çalıştırılacak dosyası.
pub fn portable_exe(dir: &Path, spec: &PlatformSpec, id: &str) -> PathBuf {
    match spec.method {
        Method::Zip => dir.join(spec.run.as_deref().unwrap_or_default()),
        _ => dir.join(spec.app.clone().unwrap_or_else(|| format!("{id}.exe"))),
    }
}

/// Çalıştırılan dosyadan uygulamanın klasörü (zip'te `run` alt klasör içerebilir).
pub fn portable_dir_of(exe: &Path, spec: &PlatformSpec) -> Option<PathBuf> {
    let depth = match spec.method {
        Method::Zip => Path::new(spec.run.as_deref().unwrap_or_default()).components().count(),
        _ => 1,
    };
    exe.ancestors().nth(depth).map(Path::to_path_buf)
}

/// Kurulu kopyanın klasörü: önce launcher'ın kaydettiği yer, sonra varsayılan klasör.
/// Çalıştırılacak dosya orada yoksa kurulu sayılmaz.
fn portable_location(record: Option<&Path>, default: &Path, spec: &PlatformSpec, id: &str, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    record.into_iter().chain([default]).find(|d| exists(&portable_exe(d, spec, id))).map(Path::to_path_buf)
}

/// Başlat menüsündeki kısayol: launcher adında bir klasörün içinde uygulamanın adı.
pub fn start_menu_shortcut(app: &App) -> PathBuf {
    let name: String = app.name.chars().filter(|c| !"<>:\"/\\|?*".contains(*c) && !c.is_control()).collect();
    let brand: String = crate::github::BRAND.name.chars().filter(|c| !"<>:\"/\\|?*".contains(*c)).collect();
    dirs::data_dir()
        .unwrap_or_default()
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join(brand.trim())
        .join(format!("{}.lnk", name.trim()))
}

/// Chrome eklentisinin klasörü: ilk kurulduğu yer hiç değişmez, çünkü Chrome onu oradan okur.
pub fn extension_dir(store: &Store, id: &str) -> PathBuf {
    match store.record(id).and_then(|r| r.location) {
        Some(loc) => PathBuf::from(loc),
        None => store.dir("extensions").join(id),
    }
}

/// Inno kaydında açılacak dosya: kurulumun bildirdiği simge dosyası, yoksa kurulum
/// klasöründeki `app` (katalogda verildiyse).
#[cfg_attr(not(windows), allow(dead_code))]
fn inno_target(display_icon: Option<String>, install_location: Option<String>, app: Option<&str>) -> Option<PathBuf> {
    // DisplayIcon "C:\...\App.exe,0" biçiminde olabilir
    display_icon
        .map(|s| PathBuf::from(s.rsplit_once(',').map_or(s.as_str(), |(p, _)| p).trim_matches('"')))
        .or_else(|| Some(PathBuf::from(install_location?.trim_matches('"')).join(app?)))
}

#[cfg(windows)]
fn inno(app_id: &str, app: Option<&str>) -> Option<Installed> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
    use winreg::RegKey;

    let path = format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{app_id}_is1");
    let places = [
        (HKEY_CURRENT_USER, KEY_READ),
        (HKEY_LOCAL_MACHINE, KEY_READ | KEY_WOW64_64KEY),
        (HKEY_LOCAL_MACHINE, KEY_READ | KEY_WOW64_32KEY),
    ];
    for (hive, flags) in places {
        let Ok(key) = RegKey::predef(hive).open_subkey_with_flags(&path, flags) else { continue };
        let get = |name: &str| key.get_value::<String, _>(name).ok().filter(|s| !s.is_empty());
        let exe = inno_target(get("DisplayIcon"), get("InstallLocation"), app);
        let uninstall = get("QuietUninstallString").or_else(|| get("UninstallString").map(|u| format!("{u} /VERYSILENT /SUPPRESSMSGBOXES /NORESTART")));
        return Some(Installed { version: get("DisplayVersion"), target: exe, uninstall, portable: false });
    }
    None
}

#[cfg(not(windows))]
fn inno(_app_id: &str, _app: Option<&str>) -> Option<Installed> {
    None
}

#[cfg(target_os = "macos")]
fn dmg(app: &str) -> Option<Installed> {
    let mut places = vec![PathBuf::from("/Applications").join(app)];
    if let Some(home) = dirs::home_dir() {
        places.push(home.join("Applications").join(app));
    }
    let path = places.into_iter().find(|p| p.exists())?;
    Some(Installed { version: bundle_version(&path), target: Some(path), ..Default::default() })
}

#[cfg(not(target_os = "macos"))]
fn dmg(_app: &str) -> Option<Installed> {
    None
}

/// macOS uygulama paketinin sürümü (Info.plist → CFBundleShortVersionString).
fn bundle_version(app: &std::path::Path) -> Option<String> {
    std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(target_os = "linux")]
fn deb(package: &str) -> Option<Installed> {
    let out = std::process::Command::new("dpkg-query")
        .args(["-W", "-f=${db:Status-Abbrev}|${Version}", package])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (status, version) = text.split_once('|')?;
    if !status.starts_with("ii") {
        return None;
    }
    Some(Installed { version: Some(debian_upstream(version)), ..Default::default() })
}

#[cfg(not(target_os = "linux"))]
fn deb(_package: &str) -> Option<Installed> {
    None
}

/// "1:2.9.0-1ubuntu2" → "2.9.0": dönem ve Debian revizyonu sürüm karşılaştırmasını bozmasın.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn debian_upstream(v: &str) -> String {
    let v = v.trim();
    let v = v.split_once(':').map_or(v, |(_, rest)| rest);
    match v.rsplit_once('-') {
        Some((up, rev)) if rev.starts_with(|c: char| c.is_ascii_digit()) => up.to_string(),
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inno_target_falls_back_to_install_folder() {
        let icon = Some(r#""C:\P\Apppp.exe",0"#.to_string());
        let loc = Some(r"C:\P\App\".to_string());
        assert_eq!(inno_target(icon, loc.clone(), Some("x.jar")), Some(PathBuf::from(r"C:\P\Apppp.exe")));
        // simge kaydı boşsa kurulum klasörü + katalogdaki app
        assert_eq!(inno_target(None, loc.clone(), Some("Control_Terminal.jar")), Some(PathBuf::from(r"C:\P\App\").join("Control_Terminal.jar")));
        assert_eq!(inno_target(None, loc, None), None);
        assert_eq!(inno_target(None, None, Some("a.exe")), None);
    }

    fn exe_spec() -> PlatformSpec {
        serde_json::from_str(r#"{"method":"exe","asset":"Tool-v*.exe","app":"Tool.exe"}"#).unwrap()
    }

    #[test]
    fn portable_location_order() {
        let spec = exe_spec();
        let (rec, def) = (Path::new("C:/rec"), Path::new("C:/def"));
        let only = |hit: &'static str| move |p: &Path| p == Path::new(hit);
        assert_eq!(portable_location(Some(rec), def, &spec, "tool", only("C:/rec/Tool.exe")), Some(rec.to_path_buf()));
        assert_eq!(portable_location(Some(rec), def, &spec, "tool", only("C:/def/Tool.exe")), Some(def.to_path_buf()));
        assert_eq!(portable_location(None, def, &spec, "tool", only("C:/def/Tool.exe")), Some(def.to_path_buf()));
        assert_eq!(portable_location(Some(rec), def, &spec, "tool", |_| false), None);
    }

    #[test]
    fn portable_exe_paths() {
        let zip: PlatformSpec = serde_json::from_str(r#"{"method":"zip","asset":"t.zip","run":"bin/tool.exe"}"#).unwrap();
        assert_eq!(portable_exe(Path::new("C:/a"), &zip, "t"), Path::new("C:/a/bin/tool.exe"));
        let bare: PlatformSpec = serde_json::from_str(r#"{"method":"exe","asset":"t.exe"}"#).unwrap();
        assert_eq!(portable_exe(Path::new("C:/a"), &bare, "tool"), Path::new("C:/a/tool.exe"));
        // klasör, çalıştırılan dosyadan geri bulunur
        assert_eq!(portable_dir_of(Path::new("C:/a/bin/tool.exe"), &zip).unwrap(), Path::new("C:/a"));
        assert_eq!(portable_dir_of(Path::new("C:/a/tool.exe"), &bare).unwrap(), Path::new("C:/a"));
    }

    #[test]
    fn strips_debian_revision() {
        assert_eq!(super::debian_upstream("1:2.9.0-1ubuntu2"), "2.9.0");
        assert_eq!(super::debian_upstream("0.1.0"), "0.1.0");
        assert_eq!(super::debian_upstream("1.2.0-beta.1"), "1.2.0-beta.1");
    }
}
