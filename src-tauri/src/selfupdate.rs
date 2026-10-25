//! C Launcher'ın kendini güncellemesi: diğer uygulamalarla aynı yol. Yeni sürüm kendi
//! release'inden indirilir, SHA256SUMS.txt ile doğrulanır, launcher nasıl kurulduysa o
//! biçimle yerine konur ve yeniden başlatılır.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use launcher_core::select;

use crate::github::Release;
use crate::install::{self, Ctx};

/// Bu kopyanın nasıl kurulduğu.
enum Kind {
    /// Windows: NSIS kurulumu (kullanıcı başına)
    WindowsSetup,
    /// macOS: .app paketinin yolu
    MacApp(PathBuf),
    /// Linux: /usr altına .deb ile
    Deb,
    /// Linux: AppImage dosyasının yolu
    AppImage(PathBuf),
    /// Linux: bir klasöre açılmış taşınabilir paket; çalıştırılabilir dosyanın yolu
    Portable(PathBuf),
}

fn kind() -> Result<Kind, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if cfg!(windows) {
        return Ok(Kind::WindowsSetup);
    }
    if cfg!(target_os = "macos") {
        // .../C Launcher.app/Contents/MacOS/c-launcher
        let app = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).ok_or("uygulama paketi bulunamadı")?;
        return Ok(Kind::MacApp(app.to_path_buf()));
    }
    if let Some(image) = std::env::var_os("APPIMAGE") {
        return Ok(Kind::AppImage(PathBuf::from(image)));
    }
    if exe.starts_with("/usr/") {
        return Ok(Kind::Deb);
    }
    Ok(Kind::Portable(exe))
}

fn pattern(kind: &Kind) -> &'static str {
    match kind {
        Kind::WindowsSetup => "*_x64-setup.exe",
        Kind::MacApp(_) => "*_universal.dmg",
        Kind::Deb => "*_amd64.deb",
        Kind::AppImage(_) => "*_amd64.AppImage",
        Kind::Portable(_) => "c-launcher-*-linux-x86_64.tar.gz",
    }
}

/// Yeni sürümü indirir, doğrular ve kurar; `relaunch` ise yeni sürümü başlatmayı da ayarlar.
/// Başarıda çağıran uygulamayı hemen kapatmalıdır.
pub async fn run(release: &Release, ctx: &Ctx<'_>, relaunch: bool) -> Result<(), String> {
    let kind = kind()?;
    let pat = pattern(&kind);
    let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
    let name = select::pick(pat, &names).ok_or_else(|| format!("release'te {pat} kalıbına uyan dosya yok"))?;
    let asset = release.assets.iter().find(|a| a.name == name).unwrap();
    let file = ctx.store.dir("downloads").join(&asset.name);
    let digest = install::download(ctx, &asset.browser_download_url, &file, asset.size).await?;
    // launcher'ın kendisi doğrulanmadan asla değiştirilmez
    if !install::verify(ctx, release, &asset.name, &digest).await? {
        let _ = std::fs::remove_file(&file);
        return Err("bu sürümün SHA256SUMS.txt dosyası henüz yayımlanmamış; birkaç dakika sonra yeniden deneyin".into());
    }
    (ctx.progress)("install", 0, 0);
    install::blocking(move || apply(kind, &file, relaunch)).await
}

fn apply(kind: Kind, file: &Path, relaunch: bool) -> Result<(), String> {
    let start = |cmd: &mut Command| if relaunch { spawn(cmd) } else { Ok(()) };
    match kind {
        Kind::WindowsSetup => windows_setup(file, relaunch),
        Kind::MacApp(app) => {
            let parent = app.parent().ok_or("uygulama klasörü bulunamadı")?.to_path_buf();
            let name = app.file_name().and_then(|n| n.to_str()).ok_or("uygulama adı okunamadı")?.to_string();
            install::install_dmg(file, &name, Some(parent))?;
            // eski süreç kapandıktan sonra yenisini aç
            start(Command::new("sh").args(["-c", "sleep 1; open \"$0\""]).arg(&app))
        }
        Kind::Deb => {
            let path = file.to_string_lossy();
            let out = install::pkexec(&["apt-get", "install", "-y", "--allow-downgrades", &path])?;
            if !out.status.success() {
                return Err(format!("paket kurulamadı: {}", install::output_tail(&out)));
            }
            start(&mut Command::new("/usr/bin/c-launcher"))
        }
        Kind::AppImage(image) => {
            replace_file(file, &image)?;
            start(&mut Command::new(&image))
        }
        Kind::Portable(exe) => {
            let tmp = file.with_extension("unpacked");
            let _ = std::fs::remove_dir_all(&tmp);
            install::extract_tar_gz(file, &tmp)?;
            let new_exe = tmp.join("c-launcher").join("c-launcher");
            if !new_exe.exists() {
                return Err("pakette c-launcher yok".into());
            }
            replace_file(&new_exe, &exe)?;
            if let (Some(dir), true) = (exe.parent(), tmp.join("c-launcher/c-launcher.png").exists()) {
                let _ = std::fs::copy(tmp.join("c-launcher/c-launcher.png"), dir.join("c-launcher.png"));
            }
            let _ = std::fs::remove_dir_all(&tmp);
            start(&mut Command::new(&exe))
        }
    }
}

/// Çalışan dosyanın yerine yenisini koyar: önce yanına kopyalar, sonra tek adımda değiştirir.
fn replace_file(new: &Path, target: &Path) -> Result<(), String> {
    let staging = target.with_file_name(format!(".{}.new", target.file_name().and_then(|n| n.to_str()).unwrap_or("c-launcher")));
    std::fs::copy(new, &staging).map_err(|e| format!("yeni sürüm yazılamadı: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&staging, target).map_err(|e| format!("yeni sürüm yerine konamadı: {e}"))
}

fn spawn(cmd: &mut Command) -> Result<(), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map(|_| ()).map_err(|e| format!("yeniden başlatılamadı: {e}"))
}

/// Windows'ta çalışan .exe değiştirilemez: launcher kapandıktan sonra kurulumu sessizce
/// çalıştırıp yeni sürümü açan ayrı bir komut başlatılır.
#[cfg(windows)]
fn windows_setup(file: &Path, relaunch: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // ping: penceresiz ortamda da çalışan bekleme
    let mut script = format!("ping -n 3 127.0.0.1 >nul & \"{}\" /S", file.display());
    if relaunch {
        script.push_str(&format!(" & start \"\" \"{}\"", exe.display()));
    }
    Command::new("cmd")
        .raw_arg(format!("/C \"{script}\""))
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("kurulum başlatılamadı: {e}"))
}

#[cfg(not(windows))]
fn windows_setup(_file: &Path, _relaunch: bool) -> Result<(), String> {
    Err("yalnızca Windows'ta".into())
}
