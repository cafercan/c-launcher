//! Bir uygulamanın bu bilgisayarda kurulu olup olmadığını ve sürümünü bulur.
//! Elle kurulmuş uygulamalar da bulunur: launcher'ın kendi kaydına güvenilmez,
//! mümkün olan her yerde sistemin kendisine bakılır.

use std::path::PathBuf;

use launcher_core::{Method, PlatformSpec};

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
        Method::Inno => inno(spec.inno_app_id.as_deref()?),
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
            let on_disk = spec
                .detect_path
                .as_deref()
                .zip(dirs::home_dir())
                .map(|(p, home)| launcher_core::paths::expand_home(p, &home).exists());
            match (record, on_disk) {
                // kayıt var ama araç diskten silinmiş: kurulu değil
                (Some(_), Some(false)) => None,
                (Some(r), _) => Some(Installed { version: Some(r.version), ..Default::default() }),
                (None, Some(true)) => Some(Installed::default()),
                _ => None,
            }
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

/// Chrome eklentisinin klasörü: ilk kurulduğu yer hiç değişmez, çünkü Chrome onu oradan okur.
pub fn extension_dir(store: &Store, id: &str) -> PathBuf {
    match store.record(id).and_then(|r| r.location) {
        Some(loc) => PathBuf::from(loc),
        None => store.dir("extensions").join(id),
    }
}

#[cfg(windows)]
fn inno(app_id: &str) -> Option<Installed> {
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
        // DisplayIcon "C:\...\App.exe,0" biçiminde olabilir
        let exe = get("DisplayIcon").map(|s| PathBuf::from(s.rsplit_once(',').map_or(s.as_str(), |(p, _)| p).trim_matches('"')));
        let uninstall = get("QuietUninstallString").or_else(|| get("UninstallString").map(|u| format!("{u} /VERYSILENT /SUPPRESSMSGBOXES /NORESTART")));
        return Some(Installed { version: get("DisplayVersion"), target: exe, uninstall, portable: false });
    }
    None
}

#[cfg(not(windows))]
fn inno(_app_id: &str) -> Option<Installed> {
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
    #[test]
    fn strips_debian_revision() {
        assert_eq!(super::debian_upstream("1:2.9.0-1ubuntu2"), "2.9.0");
        assert_eq!(super::debian_upstream("0.1.0"), "0.1.0");
        assert_eq!(super::debian_upstream("1.2.0-beta.1"), "1.2.0-beta.1");
    }
}
