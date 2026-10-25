//! İndirme, doğrulama ve yönteme göre kurma / kaldırma.
//!
//! Güncelleme, kurulumun aynısıdır: uygulama hangi biçimle kurulduysa yeni sürüm de
//! aynı biçimle, aynı yere kurulur. Uygulamanın kimliği (Inno AppId, macOS paket
//! kimliği, dpkg paket adı) değişmediği için sistemdeki varsayılan uygulama ve dosya
//! ilişkilendirme ayarları korunur.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use launcher_core::{paths, select, sums, App, Method, PlatformSpec};
use sha2::{Digest, Sha256};

use crate::detect;
use crate::github::Release;
use crate::store::{Record, Store};

/// İlerleme bildirimi: aşama, indirilen bayt, toplam bayt
pub type Progress<'a> = &'a (dyn Fn(&str, u64, u64) + Send + Sync);

pub struct Ctx<'a> {
    pub http: &'a reqwest::Client,
    pub store: &'a Store,
    pub cancel: Arc<AtomicBool>,
    pub progress: Progress<'a>,
}

/// Kurar ya da günceller; başarıda kullanıcıya gösterilecek kısa bir not döner.
pub async fn install(app: &App, spec: &PlatformSpec, release: &Release, ctx: &Ctx<'_>) -> Result<String, String> {
    let version = release.tag_name.trim_start_matches(['v', 'V']).to_string();
    match spec.method {
        Method::Script => install_script(app, spec, release, &version, ctx).await,
        method => {
            let pattern = spec.asset.as_deref().ok_or("katalogda dosya kalıbı yok")?;
            let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
            let name = select::pick(pattern, &names).ok_or_else(|| format!("release'te {pattern} kalıbına uyan dosya yok"))?;
            let asset = release.assets.iter().find(|a| a.name == name).unwrap();
            let file = ctx.store.dir("downloads").join(&asset.name);
            let digest = download(ctx, &asset.browser_download_url, &file, asset.size).await?;
            let verified = verify(ctx, release, &asset.name, &digest).await?;
            (ctx.progress)("install", 0, 0);
            let note = match method {
                Method::Inno => blocking(move || run_inno(&file)).await?,
                Method::Dmg => {
                    let app_name = spec.app.clone().unwrap_or_default();
                    blocking(move || install_dmg(&file, &app_name)).await?
                }
                Method::Deb => blocking(move || install_deb(&file)).await?,
                Method::Chrome => install_chrome(&app.id, &file, &version, ctx.store)?,
                Method::Script => unreachable!(),
            };
            let _ = std::fs::remove_file(ctx.store.dir("downloads").join(&asset.name));
            Ok(if verified { note } else { format!("{note} (release'te SHA256SUMS.txt olmadığı için doğrulanmadı)") })
        }
    }
}

/// Dosyayı indirir, indirirken SHA-256 özetini hesaplar.
async fn download(ctx: &Ctx<'_>, url: &str, dest: &Path, size_hint: u64) -> Result<String, String> {
    let resp = ctx
        .http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("indirilemedi: {e}"))?;
    let total = resp.content_length().unwrap_or(size_hint);
    let part = dest.with_extension("part");
    let mut out = std::fs::File::create(&part).map_err(|e| format!("dosya yazılamadı: {e}"))?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if ctx.cancel.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Err("iptal edildi".into());
        }
        let chunk = chunk.map_err(|e| format!("bağlantı koptu: {e}"))?;
        out.write_all(&chunk).map_err(|e| format!("dosya yazılamadı: {e}"))?;
        hasher.update(&chunk);
        done += chunk.len() as u64;
        (ctx.progress)("download", done, total);
    }
    out.flush().map_err(|e| e.to_string())?;
    drop(out);
    std::fs::rename(&part, dest).map_err(|e| e.to_string())?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Release'te SHA256SUMS.txt varsa özeti karşılaştırır. Dosya yoksa `Ok(false)`.
async fn verify(ctx: &Ctx<'_>, release: &Release, name: &str, digest: &str) -> Result<bool, String> {
    let Some(sums_asset) = release.assets.iter().find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS.txt")) else {
        return Ok(false);
    };
    (ctx.progress)("verify", 0, 0);
    let text = ctx
        .http
        .get(&sums_asset.browser_download_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("SHA256SUMS.txt indirilemedi: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    match sums::parse(&text).get(name) {
        Some(expected) if expected == digest => Ok(true),
        Some(_) => Err(format!("{name} doğrulanamadı: SHA-256 özeti tutmuyor, dosya kurulmadı")),
        None => Err(format!("{name} SHA256SUMS.txt içinde yok, dosya kurulmadı")),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

fn output_tail(out: &std::process::Output) -> String {
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(4)..].join(" · ")
}

// ------------------------------------------------------------------ Windows: Inno Setup

fn run_inno(file: &Path) -> Result<String, String> {
    let status = Command::new(file)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/CLOSEAPPLICATIONS"])
        .status()
        .map_err(|e| format!("kurulum başlatılamadı: {e}"))?;
    match status.code() {
        Some(0) => Ok("Kuruldu".into()),
        // Inno: 2 = kullanıcı iptal etti (ör. yönetici izni verilmedi)
        Some(2) | Some(5) => Err("kurulum iptal edildi ya da yönetici izni verilmedi".into()),
        code => Err(format!("kurulum başarısız (çıkış kodu {code:?})")),
    }
}

// ------------------------------------------------------------------ macOS: .dmg

fn install_dmg(file: &Path, app_name: &str) -> Result<String, String> {
    let mount = std::env::temp_dir().join(format!("c-launcher-dmg-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&mount);
    let out = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"])
        .arg(&mount)
        .arg(file)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("DMG açılamadı: {}", output_tail(&out)));
    }
    let result = copy_app(&mount, app_name);
    let _ = Command::new("hdiutil").args(["detach", "-force"]).arg(&mount).output();
    let _ = std::fs::remove_dir(&mount);
    result
}

fn copy_app(mount: &Path, app_name: &str) -> Result<String, String> {
    let src = mount.join(app_name);
    if !src.exists() {
        return Err(format!("DMG içinde {app_name} yok"));
    }
    // daha önce nereye kurulduysa oraya; yoksa yazılabiliyorsa /Applications
    let existing = detect_app_dir(app_name);
    let dir = existing.unwrap_or_else(|| {
        let system = PathBuf::from("/Applications");
        let probe = system.join(".c-launcher-write-test");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            system
        } else {
            let user = dirs::home_dir().unwrap_or_default().join("Applications");
            let _ = std::fs::create_dir_all(&user);
            user
        }
    });
    let dest = dir.join(app_name);
    let staging = dir.join(format!(".{app_name}.c-launcher-new"));
    let _ = std::fs::remove_dir_all(&staging);
    let out = Command::new("ditto").arg(&src).arg(&staging).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("uygulama kopyalanamadı: {}", output_tail(&out)));
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| format!("eski sürüm kaldırılamadı (uygulama açık olabilir): {e}"))?;
    }
    std::fs::rename(&staging, &dest).map_err(|e| e.to_string())?;
    let _ = Command::new("xattr").args(["-dr", "com.apple.quarantine"]).arg(&dest).output();
    Ok(format!("{} klasörüne kuruldu", dir.display()))
}

fn detect_app_dir(app_name: &str) -> Option<PathBuf> {
    let mut dirs_ = vec![PathBuf::from("/Applications")];
    dirs_.extend(dirs::home_dir().map(|h| h.join("Applications")));
    dirs_.into_iter().find(|d| d.join(app_name).exists())
}

// ------------------------------------------------------------------ Linux: .deb

fn pkexec(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("pkexec").args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "pkexec bulunamadı; paketi terminalden kurun".to_string()
        } else {
            e.to_string()
        }
    })
}

fn install_deb(file: &Path) -> Result<String, String> {
    let path = file.to_string_lossy();
    let out = pkexec(&["apt-get", "install", "-y", "--allow-downgrades", &path])?;
    match out.status.code() {
        Some(0) => Ok("Kuruldu".into()),
        Some(126) | Some(127) => Err("yönetici izni verilmedi".into()),
        _ => Err(format!("paket kurulamadı: {}", output_tail(&out))),
    }
}

// ------------------------------------------------------------------ Chrome eklentisi

fn install_chrome(id: &str, zip_file: &Path, version: &str, store: &Store) -> Result<String, String> {
    let dir = detect::extension_dir(store, id);
    let staging = dir.with_file_name(format!("{id}.new"));
    let _ = std::fs::remove_dir_all(&staging);
    extract_zip(zip_file, &staging, Strip::ToManifest)?;
    if !staging.join("manifest.json").exists() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("arşivde manifest.json yok, bu bir Chrome eklentisi değil".into());
    }
    let was_installed = dir.join("manifest.json").exists();
    // Chrome eklentiyi bu klasörden okur: klasörün yolu hiç değişmemeli
    let old = dir.with_file_name(format!("{id}.old"));
    let _ = std::fs::remove_dir_all(&old);
    if dir.exists() {
        std::fs::rename(&dir, &old).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&staging, &dir).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    store.set_record(id, Some(Record { version: version.to_string(), needs_reload: was_installed }));
    Ok(if was_installed {
        "Güncellendi. Chrome'da chrome://extensions sayfasında eklentinin yenile düğmesine basın".into()
    } else {
        "Dosyalar hazır. Chrome'a yüklemek için Aç'a basın".into()
    })
}

// ------------------------------------------------------------------ terminal araçları

async fn install_script(app: &App, spec: &PlatformSpec, release: &Release, version: &str, ctx: &Ctx<'_>) -> Result<String, String> {
    let url = release.zipball_url.clone().ok_or("release'in kaynak arşivi yok")?;
    let zip_file = ctx.store.dir("downloads").join(format!("{}-{version}-source.zip", app.id));
    download(ctx, &url, &zip_file, 0).await?;
    (ctx.progress)("install", 0, 0);
    let src = ctx.store.dir("tools").join(&app.id);
    let _ = std::fs::remove_dir_all(&src);
    extract_zip(&zip_file, &src, Strip::FirstDir)?;
    let _ = std::fs::remove_file(&zip_file);
    let script = spec.install.clone().unwrap_or_default();
    let src2 = src.clone();
    blocking(move || run_script(&src2, &script)).await?;
    ctx.store.set_record(&app.id, Some(Record { version: version.to_string(), needs_reload: false }));
    Ok(match &app.usage {
        Some(u) => format!("Kuruldu. Yeni bir terminal açın. {u}"),
        None => "Kuruldu. Yeni bir terminal açın".into(),
    })
}

fn run_script(dir: &Path, script: &str) -> Result<(), String> {
    if !paths::is_plain_relative(script) {
        return Err("geçersiz betik yolu".into());
    }
    let path = dir.join(script);
    if !path.exists() {
        return Err(format!("depoda {script} yok"));
    }
    let outputs: Vec<std::process::Output> = if script.ends_with(".ps1") {
        // Windows PowerShell 5.1 ve PowerShell 7 modülleri ayrı klasörlerde tutar: ikisine de kur
        let mut shells = vec!["powershell"];
        if which("pwsh") {
            shells.push("pwsh");
        }
        shells
            .into_iter()
            .map(|sh| {
                Command::new(sh)
                    .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
                    .arg(&path)
                    .current_dir(dir)
                    .output()
                    .map_err(|e| format!("{sh} çalıştırılamadı: {e}"))
            })
            .collect::<Result<_, _>>()?
    } else {
        vec![Command::new("bash").arg(&path).current_dir(dir).output().map_err(|e| format!("bash çalıştırılamadı: {e}"))?]
    };
    match outputs.iter().find(|o| !o.status.success()) {
        Some(o) => Err(format!("kurulum betiği başarısız: {}", output_tail(o))),
        None => Ok(()),
    }
}

fn which(cmd: &str) -> bool {
    let probe = if cfg!(windows) { "where" } else { "which" };
    Command::new(probe).arg(cmd).output().map(|o| o.status.success()).unwrap_or(false)
}

// ------------------------------------------------------------------ zip

#[derive(Clone, Copy, PartialEq)]
enum Strip {
    /// GitHub kaynak arşivi: en üstteki "depo-sürüm/" klasörünü at
    FirstDir,
    /// manifest.json tek bir klasörün içindeyse o klasörü kök yap
    ToManifest,
}

fn extract_zip(file: &Path, dest: &Path, strip: Strip) -> Result<(), String> {
    let f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("arşiv bozuk: {e}"))?;
    let names: Vec<String> = (0..zip.len()).filter_map(|i| zip.by_index(i).ok().map(|e| e.name().replace('\\', "/"))).collect();
    let prefix: Option<String> = match strip {
        Strip::FirstDir => None,
        Strip::ToManifest => {
            if names.iter().any(|n| n == "manifest.json") {
                Some(String::new())
            } else {
                names.iter().find(|n| n.matches('/').count() == 1 && n.ends_with("/manifest.json")).map(|n| n.trim_end_matches("manifest.json").to_string())
            }
        }
    };
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        let rel = match strip {
            Strip::FirstDir => match paths::strip_first_dir(&name) {
                Some(r) => r.to_string(),
                None => continue,
            },
            Strip::ToManifest => match prefix.as_deref() {
                Some(p) => match name.strip_prefix(p) {
                    Some(r) if !r.is_empty() => r.to_string(),
                    _ => continue,
                },
                None => name.clone(),
            },
        };
        let Some(target) = paths::safe_join(dest, &rel) else {
            return Err(format!("arşivde güvensiz yol: {name}"));
        };
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode & 0o755));
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ kaldırma

pub fn uninstall(app: &App, spec: &PlatformSpec, store: &Store) -> Result<String, String> {
    let found = detect::installed(&app.id, spec, store);
    let result = match spec.method {
        Method::Inno => {
            let cmd = found.and_then(|i| i.uninstall).ok_or("kaldırma komutu bulunamadı")?;
            run_windows_command(&cmd)
        }
        Method::Dmg => {
            let path = found.and_then(|i| i.target).ok_or("uygulama bulunamadı")?;
            // önce Çöp Kutusu'na taşımayı dene; geri alınabilir
            let script = format!("tell application \"Finder\" to delete POSIX file \"{}\"", path.display().to_string().replace('"', "\\\""));
            let trashed = Command::new("osascript").args(["-e", &script]).output().map(|o| o.status.success()).unwrap_or(false);
            if trashed {
                Ok("Çöp Kutusu'na taşındı".into())
            } else {
                std::fs::remove_dir_all(&path).map(|_| "Kaldırıldı".into()).map_err(|e| e.to_string())
            }
        }
        Method::Deb => {
            let package = spec.package.as_deref().unwrap_or_default();
            let out = pkexec(&["apt-get", "remove", "-y", package])?;
            match out.status.code() {
                Some(0) => Ok("Kaldırıldı".into()),
                Some(126) | Some(127) => Err("yönetici izni verilmedi".into()),
                _ => Err(format!("kaldırılamadı: {}", output_tail(&out))),
            }
        }
        Method::Script => {
            let src = store.dir("tools").join(&app.id);
            match spec.uninstall.as_deref() {
                Some(script) if src.join(script).exists() => run_script(&src, script).map(|_| "Kaldırıldı".to_string()),
                Some(_) => Err("kaldırma betiği bulunamadı; aracı önce launcher ile yeniden kurun".into()),
                None => Err("bu araç için kaldırma betiği tanımlı değil".into()),
            }
            .inspect(|_| {
                let _ = std::fs::remove_dir_all(&src);
            })
        }
        Method::Chrome => {
            let dir = detect::extension_dir(store, &app.id);
            std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            Ok("Dosyalar silindi. Chrome'daki eklentiyi de chrome://extensions sayfasından kaldırın".into())
        }
    };
    if result.is_ok() {
        store.set_record(&app.id, None);
    }
    result
}

#[cfg(windows)]
fn run_windows_command(cmd: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let status = Command::new("cmd").raw_arg("/C ").raw_arg(cmd).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok("Kaldırıldı".into())
    } else {
        Err(format!("kaldırılamadı (çıkış kodu {:?})", status.code()))
    }
}

#[cfg(not(windows))]
fn run_windows_command(_cmd: &str) -> Result<String, String> {
    Err("yalnızca Windows'ta".into())
}
