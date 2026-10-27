//! Uygulamaları, web sayfalarını ve Chrome'un eklentiler sayfasını açar.

use std::path::Path;
use std::process::{Command, Stdio};

use launcher_core::{Method, PlatformSpec};

use crate::detect::Installed;

fn spawn(cmd: &mut Command) -> Result<(), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Yalnızca GitHub sayfaları açılır: katalogdan gelen adres başka bir şeyi çalıştıramaz.
pub fn open_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://github.com/") || url.contains(['"', '\'', ' ', '&', '|', '^', '<', '>']) {
        return Err("yalnızca github.com adresleri açılabilir".into());
    }
    if cfg!(windows) {
        spawn(Command::new("explorer").arg(url))
    } else if cfg!(target_os = "macos") {
        spawn(Command::new("open").arg(url))
    } else {
        spawn(Command::new("xdg-open").arg(url))
    }
}

/// Kurulu bir uygulamayı başlatır.
pub fn open_app(spec: &PlatformSpec, installed: &Installed) -> Result<(), String> {
    match spec.method {
        Method::Inno => {
            let exe = installed.target.as_deref().filter(|p| p.exists()).ok_or("uygulamanın çalıştırılabilir dosyası bulunamadı")?;
            // exe olmayan dosya (ör. .jar) Windows'taki dosya ilişkilendirmesiyle açılır
            if !exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
                return spawn(Command::new("explorer").arg(exe));
            }
            spawn(Command::new(exe).current_dir(exe.parent().unwrap_or(Path::new("."))))
        }
        Method::Dmg => {
            let app = installed.target.as_deref().ok_or("uygulama bulunamadı")?;
            spawn(Command::new("open").arg(app))
        }
        Method::Exe | Method::Zip => {
            let exe = installed.target.as_deref().filter(|p| p.exists()).ok_or("uygulamanın çalıştırılabilir dosyası bulunamadı")?;
            spawn(Command::new(exe).current_dir(exe.parent().unwrap_or(Path::new("."))))
        }
        Method::Deb if installed.portable => {
            let run = installed.target.as_deref().ok_or("uygulama bulunamadı")?;
            spawn(Command::new(run).current_dir(run.parent().unwrap_or(Path::new("."))))
        }
        Method::Deb => {
            let run = spec.run.as_deref().or(spec.package.as_deref()).ok_or("başlatma komutu tanımlı değil")?;
            spawn(&mut Command::new(run))
        }
        Method::Script | Method::Chrome => Err("bu tür açılmaz".into()),
    }
}

/// Chrome'da chrome://extensions sayfasını açmayı dener.
pub fn open_chrome_extensions() -> Result<(), String> {
    const PAGE: &str = "chrome://extensions";
    if cfg!(windows) {
        // "start" Chrome'u kayıtlı App Paths üzerinden bulur
        return spawn(Command::new("cmd").args(["/C", "start", "", "chrome", PAGE]));
    }
    if cfg!(target_os = "macos") {
        return spawn(Command::new("open").args(["-a", "Google Chrome", PAGE]));
    }
    for bin in ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"] {
        if spawn(Command::new(bin).arg(PAGE)).is_ok() {
            return Ok(());
        }
    }
    Err("Chrome bulunamadı".into())
}
