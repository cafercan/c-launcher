//! Arşivden çıkan ya da katalogdan gelen yolların hedef klasörün dışına taşmasını önler.

use std::path::{Component, Path, PathBuf};

/// "a/b.sh" gibi: mutlak değil, ".." ya da sürücü harfi içermiyor.
pub fn is_plain_relative(p: &str) -> bool {
    !p.is_empty() && !p.contains('\\') && Path::new(p).components().all(|c| matches!(c, Component::Normal(_)))
}

/// Zip içindeki bir girdinin hedef klasör altındaki güvenli yolu (zip-slip koruması).
/// Girdi klasörün dışına çıkacaksa `None` döner.
pub fn safe_join(base: &Path, entry: &str) -> Option<PathBuf> {
    let entry = entry.replace('\\', "/");
    let mut out = base.to_path_buf();
    let mut depth = 0usize;
    for part in entry.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            p if p.contains(':') => return None,
            p => {
                out.push(p);
                depth += 1;
            }
        }
    }
    (depth > 0).then_some(out)
}

/// GitHub'ın kaynak zip'leri her şeyi "depo-sürüm/" klasörüne koyar; o öneki atar.
pub fn strip_first_dir(entry: &str) -> Option<&str> {
    entry.split_once('/').map(|(_, rest)| rest).filter(|r| !r.is_empty())
}

/// "~/x" yolunu ev dizinine göre açar.
pub fn expand_home(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(p),
    }
}

/// Katalogdaki bir algılama yolunu açar: "~/x" ev dizinine, "{documents}/x" Belgeler
/// klasörüne göre. Windows'ta Belgeler OneDrive'a yönlendirilmiş olabilir; o yüzden
/// "~/Documents" yazmak yetmez. Klasör bilinmiyorsa `None` döner.
pub fn expand_known(p: &str, home: Option<&Path>, documents: Option<&Path>) -> Option<PathBuf> {
    if let Some(rest) = p.strip_prefix("{documents}/") {
        return documents.map(|d| d.join(rest));
    }
    if p.starts_with("~/") {
        return home.map(|h| expand_home(p, h));
    }
    Some(PathBuf::from(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_escapes() {
        let base = Path::new("/data/ext");
        assert_eq!(safe_join(base, "icons/a.png"), Some(PathBuf::from("/data/ext/icons/a.png")));
        assert_eq!(safe_join(base, "./manifest.json"), Some(PathBuf::from("/data/ext/manifest.json")));
        assert_eq!(safe_join(base, "../../etc/passwd"), None);
        assert_eq!(safe_join(base, "a/../../x"), None);
        assert_eq!(safe_join(base, "C:/Windows/x"), None);
        assert_eq!(safe_join(base, "..\\..\\x"), None);
        assert_eq!(safe_join(base, "/"), None);
        // mutlak yol klasör altına alınır, dışarı çıkamaz
        assert_eq!(safe_join(base, "/etc/passwd"), Some(PathBuf::from("/data/ext/etc/passwd")));
    }

    #[test]
    fn relative_scripts_only() {
        assert!(is_plain_relative("install.sh"));
        assert!(is_plain_relative("scripts/install.ps1"));
        assert!(!is_plain_relative("../install.sh"));
        assert!(!is_plain_relative("/bin/sh"));
        assert!(!is_plain_relative("a\\b.ps1"));
    }

    #[test]
    fn expands_known_folders() {
        let home = Path::new("/home/u");
        let docs = Path::new("/docs");
        assert_eq!(expand_known("~/.local/x", Some(home), Some(docs)), Some(home.join(".local/x")));
        assert_eq!(expand_known("{documents}/WindowsPowerShell/Modules/xd", Some(home), Some(docs)), Some(docs.join("WindowsPowerShell/Modules/xd")));
        assert_eq!(expand_known("{documents}/x", Some(home), None), None);
        assert_eq!(expand_known("~/x", None, Some(docs)), None);
    }

    #[test]
    fn strips_github_prefix() {
        assert_eq!(strip_first_dir("xd-0.2.0/install.sh"), Some("install.sh"));
        assert_eq!(strip_first_dir("xd-0.2.0/"), None);
    }
}
