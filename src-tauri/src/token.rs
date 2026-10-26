//! GitHub girişinin token'ı: Windows'ta Kimlik Bilgileri Yöneticisi'nde, macOS'te Anahtar
//! Zinciri'nde tutulur. Linux'ta (D-Bus'a bağımlı olmamak için) veri klasöründe yalnızca
//! kullanıcının okuyabildiği bir dosyada.

#[cfg(any(windows, target_os = "macos"))]
fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(&crate::github::BRAND.data_dir, "github").map_err(|e| e.to_string())
}

#[cfg(any(windows, target_os = "macos"))]
pub fn load() -> Option<String> {
    entry().ok()?.get_password().ok().filter(|t| !t.is_empty())
}

#[cfg(any(windows, target_os = "macos"))]
pub fn save(token: &str) -> Result<(), String> {
    entry()?.set_password(token).map_err(|e| format!("token saklanamadı: {e}"))
}

#[cfg(any(windows, target_os = "macos"))]
pub fn clear() {
    if let Ok(e) = entry() {
        let _ = e.delete_credential();
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn file() -> std::path::PathBuf {
    dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join(&crate::github::BRAND.data_dir).join("token")
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn load() -> Option<String> {
    std::fs::read_to_string(file()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn save(token: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("token saklanamadı: {e}"))?;
    f.write_all(token.as_bytes()).map_err(|e| format!("token saklanamadı: {e}"))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn clear() {
    let _ = std::fs::remove_file(file());
}
