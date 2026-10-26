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
use launcher_core::auth::asset_url;
use launcher_core::{paths, select, sums, App, Method, PlatformSpec};
use sha2::{Digest, Sha256};

use crate::detect;
use crate::github::{self, Release};
use crate::store::{Record, Store};

/// İlerleme bildirimi: aşama, indirilen bayt, toplam bayt
pub type Progress<'a> = &'a (dyn Fn(&str, u64, u64) + Send + Sync);

pub struct Ctx<'a> {
    pub http: &'a reqwest::Client,
    pub store: &'a Store,
    pub cancel: Arc<AtomicBool>,
    pub progress: Progress<'a>,
    /// GitHub girişinin token'ı; yalnızca API adreslerine gönderilir
    pub token: Option<String>,
}

/// Kurar ya da günceller; başarıda kullanıcıya gösterilecek kısa bir not döner.
pub async fn install(app: &App, spec: &PlatformSpec, release: &Release, ctx: &Ctx<'_>) -> Result<String, String> {
    let version = release.tag_name.trim_start_matches(['v', 'V']).to_string();
    let before = detect::installed(&app.id, spec, ctx.store);
    // seçilen klasör yalnızca yeni kurulumlarda kullanılır; güncellemeler ilk kurulduğu yerde kalır
    let root = if before.is_none() { ctx.store.install_root() } else { None };
    if spec.method == Method::Script {
        return install_script(app, spec, release, &version, ctx).await;
    }
    let portable = uses_portable(spec, before.as_ref(), ctx.store.install_root().is_some());
    let pattern = asset_pattern(spec, portable).ok_or("katalogda dosya kalıbı yok")?;
    let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
    let name = select::pick(pattern, &names).ok_or_else(|| format!("release'te {pattern} kalıbına uyan dosya yok"))?;
    let asset = release.assets.iter().find(|a| a.name == name).unwrap();
    let file = ctx.store.dir("downloads").join(&asset.name);
    let digest = download(ctx, asset_url(&asset.url, &asset.browser_download_url, app.private), &file, asset.size).await?;
    let verified = verify(ctx, release, &asset.name, &digest, app.private).await?;
    if let Err(e) = sums_required(spec.method, verified) {
        let _ = std::fs::remove_file(&file);
        return Err(e);
    }
    (ctx.progress)("install", 0, 0);
    let file2 = file.clone();
    let note = match spec.method {
        Method::Inno => {
            let dir = root.map(|r| r.join(folder_name(&app.name)));
            blocking(move || run_inno(&file2, dir)).await?
        }
        Method::Dmg => {
            let app_name = spec.app.clone().unwrap_or_default();
            // güncellemede: uygulama nerede duruyorsa oraya
            let target = before.as_ref().and_then(|i| i.target.as_ref()).and_then(|p| p.parent().map(Path::to_path_buf)).or(root.clone());
            let (note, dir) = blocking(move || install_dmg(&file2, &app_name, target)).await?;
            let custom = !matches!(dir.to_str(), Some("/Applications"));
            ctx.store.set_record(&app.id, Some(Record { version: version.clone(), location: custom.then(|| dir.display().to_string()), ..Default::default() }));
            note
        }
        Method::Deb if portable => {
            let p = spec.portable.clone().unwrap();
            let location = match before.as_ref() {
                Some(_) => ctx.store.record(&app.id).and_then(|r| r.location).map(PathBuf::from).ok_or("taşınabilir kurulumun yeri bulunamadı")?,
                None => root.clone().ok_or("kurulum klasörü seçilmemiş")?.join(folder_name(&app.name)),
            };
            let (id, name) = (app.id.clone(), app.name.clone());
            let loc2 = location.clone();
            let note = blocking(move || install_portable(&id, &name, &file2, &loc2, &p)).await?;
            ctx.store.set_record(&app.id, Some(Record { version: version.clone(), location: Some(location.display().to_string()), ..Default::default() }));
            note
        }
        Method::Deb => blocking(move || install_deb(&file2)).await?,
        Method::Chrome => {
            let dir = match (&before, &root) {
                (Some(_), _) => detect::extension_dir(ctx.store, &app.id),
                (None, Some(r)) => r.join("Chrome eklentileri").join(&app.id),
                (None, None) => ctx.store.dir("extensions").join(&app.id),
            };
            install_chrome(&app.id, &file, &version, &dir, ctx.store)?
        }
        Method::Exe | Method::Zip => {
            // güncellemede bulunduğu klasörde kalır
            let dir = match before.as_ref().and_then(|i| i.target.as_deref()) {
                Some(exe) => detect::portable_dir_of(exe, spec).ok_or("kurulum yeri bulunamadı")?,
                None => detect::portable_default_dir(ctx.store, &app.id),
            };
            let (app2, spec2, dir2) = (app.clone(), spec.clone(), dir.clone());
            let note = blocking(move || install_portable_windows(&app2, &spec2, &file2, &dir2)).await?;
            ctx.store.set_record(&app.id, Some(Record { version: version.clone(), location: Some(dir.display().to_string()), ..Default::default() }));
            note
        }
        Method::Script => unreachable!(),
    };
    let _ = std::fs::remove_file(&file);
    Ok(if verified { note } else { format!("{note} (release'te SHA256SUMS.txt olmadığı için doğrulanmadı)") })
}

/// Linux: taşınabilir paket mi kurulacak? Zaten öyle kuruluysa evet; hiç kurulu değilse
/// kullanıcı bir kurulum klasörü seçtiyse ve uygulama böyle bir paket sunuyorsa evet.
pub fn uses_portable(spec: &PlatformSpec, before: Option<&detect::Installed>, root_set: bool) -> bool {
    spec.method == Method::Deb
        && spec.portable.is_some()
        && match before {
            Some(i) => i.portable,
            None => root_set,
        }
}

pub fn asset_pattern(spec: &PlatformSpec, portable: bool) -> Option<&str> {
    if portable {
        spec.portable.as_ref().map(|p| p.asset.as_str())
    } else {
        spec.asset.as_deref()
    }
}

/// Klasör adı olarak güvenli uygulama adı.
fn folder_name(name: &str) -> String {
    let n: String = name.chars().filter(|c| !"<>:\"/\\|?*".contains(*c) && !c.is_control()).collect();
    match n.trim().trim_matches('.') {
        "" => "app".into(),
        t => t.to_string(),
    }
}

/// Dosyayı indirir, indirirken SHA-256 özetini hesaplar.
pub(crate) async fn download(ctx: &Ctx<'_>, url: &str, dest: &Path, size_hint: u64) -> Result<String, String> {
    let resp = get(ctx, url).send().await.map_err(|e| format!("indirilemedi: {e}"))?;
    github::note_status(resp.status().as_u16(), ctx.token.is_some());
    let resp = resp.error_for_status().map_err(|e| format!("indirilemedi: {e}"))?;
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

/// GitHub'dan dosya isteği: API adresine token, release dosyasının API adresine ayrıca
/// dosyanın kendisini isteyen `Accept` başlığı eklenir.
fn get(ctx: &Ctx<'_>, url: &str) -> reqwest::RequestBuilder {
    let req = github::authed(ctx.http.get(url), url, ctx.token.as_deref());
    if url.contains("/releases/assets/") {
        req.header("Accept", "application/octet-stream")
    } else {
        req
    }
}

/// Release'te SHA256SUMS.txt varsa özeti karşılaştırır. Dosya yoksa `Ok(false)`.
pub(crate) async fn verify(ctx: &Ctx<'_>, release: &Release, name: &str, digest: &str, private: bool) -> Result<bool, String> {
    let Some(sums_asset) = release.assets.iter().find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS.txt")) else {
        return Ok(false);
    };
    (ctx.progress)("verify", 0, 0);
    let text = get(ctx, asset_url(&sums_asset.url, &sums_asset.browser_download_url, private))
        .send()
        .await
        .inspect(|r| github::note_status(r.status().as_u16(), ctx.token.is_some()))
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

pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

pub(crate) fn output_tail(out: &std::process::Output) -> String {
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(4)..].join(" · ")
}

// ------------------------------------------------------------------ Windows: Inno Setup

fn run_inno(file: &Path, dir: Option<PathBuf>) -> Result<String, String> {
    let log = file.with_extension("log");
    let mut cmd = Command::new(file);
    // Açık uygulama Restart Manager ile kapatılır, kurulumdan sonra yeniden açılır.
    // Kapanma isteğine yanıt vermeyen (ör. hata penceresinde takılmış) süreç zorla
    // kapatılır; yoksa dosyaları kilitli kalır ve kurulum yarıda kesilir.
    cmd.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/CLOSEAPPLICATIONS", "/FORCECLOSEAPPLICATIONS"]);
    cmd.arg(format!("/LOG={}", log.display()));
    if let Some(d) = dir {
        cmd.arg(format!("/DIR={}", d.display()));
    }
    let status = cmd
        .status()
        .map_err(|e| format!("kurulum başlatılamadı: {e}"))?;
    match inno_exit(status.code()) {
        Ok(()) => {
            let _ = std::fs::remove_file(&log);
            Ok("Kuruldu".into())
        }
        Err(reason) => Err(format!("{reason} · ayrıntı: {}", log.display())),
    }
}

/// Inno Setup çıkış kodunun anlamı (https://jrsoftware.org/ishelp/topic_setupexitcodes.htm).
fn inno_exit(code: Option<i32>) -> Result<(), String> {
    match code {
        Some(0) => Ok(()),
        Some(2) => Err("kurulum başlamadan iptal edildi (yönetici izni verilmemiş olabilir)".into()),
        Some(5) => Err("kurulum yarıda kesildi: uygulama açık kalmış ya da dosyaları kullanımda olabilir; uygulamayı kapatıp tekrar deneyin".into()),
        Some(c @ (1 | 3 | 4)) => Err(format!("kurulum hata verdi (çıkış kodu {c})")),
        Some(6) => Err("kurulum dışarıdan sonlandırıldı".into()),
        Some(7) => Err("kurulum ön hazırlığı tamamlanamadı".into()),
        Some(8) => Err("kurulumdan önce Windows'un yeniden başlatılması gerekiyor".into()),
        code => Err(format!("kurulum başarısız (çıkış kodu {code:?})")),
    }
}

// ------------------------------------------------------------------ macOS: .dmg

pub(crate) fn install_dmg(file: &Path, app_name: &str, target: Option<PathBuf>) -> Result<(String, PathBuf), String> {
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
    let result = copy_app(&mount, app_name, target);
    let _ = Command::new("hdiutil").args(["detach", "-force"]).arg(&mount).output();
    let _ = std::fs::remove_dir(&mount);
    result
}

fn copy_app(mount: &Path, app_name: &str, target: Option<PathBuf>) -> Result<(String, PathBuf), String> {
    let src = mount.join(app_name);
    if !src.exists() {
        return Err(format!("DMG içinde {app_name} yok"));
    }
    // istenen klasör; yoksa daha önce nereye kurulduysa oraya; o da yoksa yazılabiliyorsa /Applications
    if let Some(t) = &target {
        std::fs::create_dir_all(t).map_err(|e| format!("{} oluşturulamadı: {e}", t.display()))?;
    }
    let existing = target.or_else(|| detect_app_dir(app_name));
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
    Ok((format!("{} klasörüne kuruldu", dir.display()), dir))
}

fn detect_app_dir(app_name: &str) -> Option<PathBuf> {
    let mut dirs_ = vec![PathBuf::from("/Applications")];
    dirs_.extend(dirs::home_dir().map(|h| h.join("Applications")));
    dirs_.into_iter().find(|d| d.join(app_name).exists())
}

// ------------------------------------------------------------------ Linux: .deb

pub(crate) fn pkexec(args: &[&str]) -> Result<std::process::Output, String> {
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

fn install_chrome(id: &str, zip_file: &Path, version: &str, dir: &Path, store: &Store) -> Result<String, String> {
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{} oluşturulamadı: {e}", parent.display()))?;
    }
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
        std::fs::rename(dir, &old).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&staging, dir).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    store.set_record(id, Some(Record { version: version.to_string(), needs_reload: was_installed, location: Some(dir.display().to_string()) }));
    Ok(if was_installed {
        "Güncellendi. Chrome'da chrome://extensions sayfasında eklentinin yenile düğmesine basın".into()
    } else {
        "Dosyalar hazır. Chrome'a yüklemek için Aç'a basın".into()
    })
}

// ------------------------------------------------------------------ Windows: kurulumsuz uygulama

/// Kurulumsuz yöntemlerde özet listesi zorunlu: doğrulanmamış bir .exe çalıştırılmaz.
fn sums_required(method: Method, verified: bool) -> Result<(), String> {
    match method {
        Method::Exe | Method::Zip if !verified => Err("release'te SHA256SUMS.txt yok, dosya kurulmadı".into()),
        _ => Ok(()),
    }
}

/// Dosya ya da klasör kullanımdaysa (uygulama açık) Windows'un verdiği hatayı anlaşılır kılar.
fn locked(e: std::io::Error) -> String {
    match e.raw_os_error() {
        // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
        Some(5 | 32 | 33) => "uygulama açık, kapatıp tekrar deneyin".into(),
        _ => e.to_string(),
    }
}

fn install_portable_windows(app: &App, spec: &PlatformSpec, file: &Path, dir: &Path) -> Result<String, String> {
    place_portable(&app.id, spec, file, dir)?;
    // kısayol olmasa da uygulama launcher'dan açılır: hata kurulumu bozmaz
    let exe = detect::portable_exe(dir, spec, &app.id);
    let note = match write_shortcut(&detect::start_menu_shortcut(app), &exe, dir) {
        Ok(()) => "Kuruldu; Başlat menüsünde de bulunur".to_string(),
        Err(e) => format!("Kuruldu (Başlat menüsü kısayolu eklenemedi: {e})"),
    };
    Ok(note)
}

/// İndirilen exe'yi ya da zip'in içeriğini uygulamanın klasörüne yerleştirir.
fn place_portable(id: &str, spec: &PlatformSpec, file: &Path, dir: &Path) -> Result<(), String> {
    match spec.method {
        Method::Exe => {
            std::fs::create_dir_all(dir).map_err(|e| format!("{} oluşturulamadı: {e}", dir.display()))?;
            replace_exe(file, &detect::portable_exe(dir, spec, id))?;
        }
        _ => {
            if !one_top_folder(&zip_names(file)?) {
                return Err("zip'in kökünde tek bir klasör olmalı".into());
            }
            let staging = dir.with_file_name(format!("{id}.new"));
            let _ = std::fs::remove_dir_all(&staging);
            extract_zip(file, &staging, Strip::FirstDir)?;
            let run = spec.run.as_deref().unwrap_or_default();
            if !staging.join(run).is_file() {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(format!("arşivde {run} yok"));
            }
            swap_dir(&staging, dir)?;
        }
    }
    Ok(())
}

/// Yeni .exe'yi önce yanına kopyalar, sonra eskisinin üzerine taşır: yarım kalmış bir dosya
/// hiçbir zaman uygulamanın yerinde durmaz.
fn replace_exe(new: &Path, target: &Path) -> Result<(), String> {
    let staged = target.with_extension("exe.new");
    std::fs::copy(new, &staged).map_err(|e| format!("dosya yazılamadı: {e}"))?;
    std::fs::rename(&staged, target).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        locked(e)
    })
}

/// Uygulama klasörünü siler. Önce yeniden adlandırılır: uygulama açıksa bu adım tümden
/// başarısız olur ve klasör olduğu gibi kalır; yarım silinmiş bir kurulum bırakılmaz.
fn remove_app_dir(dir: &Path) -> Result<(), String> {
    let doomed = dir.with_file_name(format!("{}.old", dir.file_name().and_then(|n| n.to_str()).unwrap_or("app")));
    let _ = std::fs::remove_dir_all(&doomed);
    std::fs::rename(dir, &doomed).map_err(locked)?;
    std::fs::remove_dir_all(&doomed).map_err(|e| e.to_string())
}

/// Zip'in kökünde tek bir klasör var mı (adı ne olursa olsun)?
fn one_top_folder(names: &[String]) -> bool {
    let mut tops = names.iter().map(|n| n.split_once('/').map(|(top, _)| top));
    match tops.next() {
        Some(Some(first)) => tops.all(|t| t == Some(first)),
        _ => false,
    }
}

fn zip_names(file: &Path) -> Result<Vec<String>, String> {
    let f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("arşiv bozuk: {e}"))?;
    (0..zip.len())
        .map(|i| zip.by_index(i).map_err(|e| e.to_string()).and_then(|e| e.name().map(|n| n.replace('\\', "/")).map_err(|e| format!("arşivde geçersiz dosya adı: {e}"))))
        .collect()
}

/// Hazırlanan klasörü eskisinin yerine koyar; ikinci adım başarısız olursa eski klasör geri gelir.
fn swap_dir(new: &Path, target: &Path) -> Result<(), String> {
    let old = target.with_file_name(format!("{}.old", target.file_name().and_then(|n| n.to_str()).unwrap_or("app")));
    let _ = std::fs::remove_dir_all(&old);
    let had_old = target.exists();
    if had_old {
        std::fs::rename(target, &old).map_err(|e| {
            let _ = std::fs::remove_dir_all(new);
            locked(e)
        })?;
    } else if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if let Err(e) = std::fs::rename(new, target) {
        if had_old {
            let _ = std::fs::rename(&old, target);
        }
        let _ = std::fs::remove_dir_all(new);
        return Err(locked(e));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Başlat menüsü kısayolu (Windows aramasında da çıkar). Yollar komut satırına değil ortam
/// değişkenlerine konur: adlardaki tırnak ya da $ işaretleri betiği bozamaz.
#[cfg(windows)]
fn write_shortcut(lnk: &Path, exe: &Path, dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    if let Some(parent) = lnk.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let script = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:CL_LNK); $s.TargetPath = $env:CL_EXE; $s.WorkingDirectory = $env:CL_DIR; $s.Save()";
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("CL_LNK", lnk)
        .env("CL_EXE", exe)
        .env("CL_DIR", dir)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() && lnk.exists() {
        Ok(())
    } else {
        Err(output_tail(&out))
    }
}

#[cfg(not(windows))]
fn write_shortcut(_lnk: &Path, _exe: &Path, _dir: &Path) -> Result<(), String> {
    Err("yalnızca Windows'ta".into())
}

// ------------------------------------------------------------------ Linux: taşınabilir paket

fn desktop_file(id: &str) -> PathBuf {
    dirs::data_dir().unwrap_or_default().join("applications").join(format!("c-launcher-{id}.desktop"))
}

fn install_portable(id: &str, name: &str, archive: &Path, location: &Path, p: &launcher_core::Portable) -> Result<String, String> {
    let staging = location.with_file_name(format!(".{}.c-launcher-new", folder_name(name)));
    let _ = std::fs::remove_dir_all(&staging);
    extract_tar_gz(archive, &staging)?;
    if !staging.join(&p.run).exists() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("paketin içinde {} yok", p.run));
    }
    if location.exists() {
        std::fs::remove_dir_all(location).map_err(|e| format!("eski sürüm kaldırılamadı: {e}"))?;
    }
    std::fs::rename(&staging, location).map_err(|e| e.to_string())?;
    // uygulama menüsünde görünsün
    let run = location.join(&p.run);
    let icon = p.icon.as_ref().map(|i| location.join(i).display().to_string()).unwrap_or_default();
    let wm = p.wm_class.as_ref().map(|w| format!("StartupWMClass={w}\n")).unwrap_or_default();
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{}\"\nIcon={icon}\nTerminal=false\nCategories=Utility;\n{wm}X-C-Launcher-Id={id}\n",
        run.display()
    );
    let desktop = desktop_file(id);
    if let Some(dir) = desktop.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&desktop, entry).map_err(|e| format!("menü kısayolu yazılamadı: {e}"))?;
    let _ = Command::new("update-desktop-database").arg(desktop.parent().unwrap()).output();
    Ok(format!("{} klasörüne kuruldu", location.display()))
}

pub(crate) fn extract_tar_gz(file: &Path, dest: &Path) -> Result<(), String> {
    let f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(f));
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in archive.entries().map_err(|e| format!("arşiv bozuk: {e}"))? {
        let mut entry = entry.map_err(|e| format!("arşiv bozuk: {e}"))?;
        let name = entry.path().map_err(|e| e.to_string())?.to_string_lossy().to_string();
        let Some(target) = paths::safe_join(dest, &name) else {
            // "./" gibi kök girdileri atla, dışarı çıkan yolları reddet
            if name.trim_start_matches("./").is_empty() {
                continue;
            }
            return Err(format!("arşivde güvensiz yol: {name}"));
        };
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else if kind.is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mode = entry.header().mode().unwrap_or(0o644);
            let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode & 0o755));
            }
            #[cfg(not(unix))]
            let _ = mode;
        }
        // bağlantılar ve özel dosyalar güvenlik için atlanır
    }
    Ok(())
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
    ctx.store.set_record(&app.id, Some(Record { version: version.to_string(), ..Default::default() }));
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
    let names: Vec<String> = (0..zip.len()).filter_map(|i| zip.by_index(i).ok().and_then(|e| e.name().ok().map(|n| n.replace('\\', "/")))).collect();
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
        // zip 9: geçersiz (UTF-8 olmayan, güvensiz) adlar hata olarak döner
        let name = entry.name().map_err(|e| format!("arşivde geçersiz dosya adı: {e}"))?.replace('\\', "/");
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
        Method::Deb if found.as_ref().is_some_and(|i| i.portable) => {
            let location = store.record(&app.id).and_then(|r| r.location).ok_or("kurulum yeri bulunamadı")?;
            std::fs::remove_dir_all(&location).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(desktop_file(&app.id));
            Ok("Kaldırıldı".into())
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
        Method::Exe | Method::Zip => {
            let exe = found.and_then(|i| i.target).ok_or("uygulama bulunamadı")?;
            let dir = detect::portable_dir_of(&exe, spec).ok_or("kurulum yeri bulunamadı")?;
            // zip'te klasörün tamamı uygulamanındır; exe'de klasörde yalnızca o dosya silinir
            match spec.method {
                Method::Zip => remove_app_dir(&dir)?,
                _ => {
                    std::fs::remove_file(&exe).map_err(locked)?;
                    let _ = std::fs::remove_dir(&dir); // boşsa
                }
            }
            let lnk = detect::start_menu_shortcut(app);
            let _ = std::fs::remove_file(&lnk);
            if let Some(folder) = lnk.parent() {
                let _ = std::fs::remove_dir(folder); // son kısayolsa launcher klasörü de gider
            }
            Ok("Kaldırıldı".into())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("c-launcher-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn portable_requires_sums() {
        assert_eq!(sums_required(Method::Exe, false).unwrap_err(), "release'te SHA256SUMS.txt yok, dosya kurulmadı");
        assert!(sums_required(Method::Zip, false).is_err());
        assert!(sums_required(Method::Zip, true).is_ok());
        // eski yöntemler eskisi gibi: uyarıyla kurulur
        assert!(sums_required(Method::Inno, false).is_ok());
    }

    /// Verilen (yol, içerik) girdileriyle bir zip dosyası yazar.
    fn make_zip(path: &Path, entries: &[(&str, &str)]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, body) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn zip_install_and_update_place_folder_contents() {
        let d = temp("place");
        let spec: PlatformSpec = serde_json::from_str(r#"{"method":"zip","asset":"tool-*-windows-x64-portable.zip","run":"tool.exe"}"#).unwrap();
        let dir = d.join("apps").join("tool");
        let v1 = d.join("v1.zip");
        make_zip(&v1, &[("tool-1.0.0-windows-x64-portable/tool.exe", "1"), ("tool-1.0.0-windows-x64-portable/lib/a.dll", "a")]);
        place_portable("tool", &spec, &v1, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("tool.exe")).unwrap(), "1");
        assert!(dir.join("lib/a.dll").exists());
        // güncelleme: klasör adı sürümle değişse de içerik aynı yere gelir, eski dosyalar gider
        let v2 = d.join("v2.zip");
        make_zip(&v2, &[("tool-2.0.0-windows-x64-portable/tool.exe", "2")]);
        place_portable("tool", &spec, &v2, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("tool.exe")).unwrap(), "2");
        assert!(!dir.join("lib").exists() && !d.join("apps/tool.new").exists() && !d.join("apps/tool.old").exists());
        // kökte tek klasör yoksa: kurulmaz, eski sürüm yerinde
        let flat = d.join("flat.zip");
        make_zip(&flat, &[("tool.exe", "3")]);
        assert!(place_portable("tool", &spec, &flat, &dir).unwrap_err().contains("tek bir klasör"));
        // run yoksa: kurulmaz, eski sürüm yerinde
        let wrong = d.join("wrong.zip");
        make_zip(&wrong, &[("x-1/other.exe", "4")]);
        assert!(place_portable("tool", &spec, &wrong, &dir).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("tool.exe")).unwrap(), "2");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_must_have_one_top_folder() {
        let v = |names: &[&str]| one_top_folder(&names.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(v(&["tool-1.0-portable/", "tool-1.0-portable/tool.exe", "tool-1.0-portable/lib/x.dll"]));
        assert!(!v(&["tool.exe", "README.md"]));
        assert!(!v(&["a/tool.exe", "b/other.exe"]));
        assert!(!v(&["a/tool.exe", "README.md"]));
        assert!(!v(&[]));
    }

    #[test]
    fn zip_uninstall_removes_whole_folder() {
        let d = temp("rm");
        let dir = d.join("tool");
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("tool.exe"), "x").unwrap();
        remove_app_dir(&dir).unwrap();
        assert!(!dir.exists() && !d.join("tool.old").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(windows)]
    #[test]
    fn zip_uninstall_keeps_folder_when_locked() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = temp("rmlock");
        let dir = d.join("tool");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("data.bin"), "veri").unwrap();
        std::fs::write(dir.join("tool.exe"), "x").unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(dir.join("tool.exe")).unwrap();
        let err = remove_app_dir(&dir).unwrap_err();
        drop(held);
        assert!(err.contains("uygulama açık"), "{err}");
        assert!(dir.join("data.bin").exists(), "kilitliyken hiçbir dosya silinmemeli");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_update_swaps_folder() {
        let d = temp("swap");
        let (target, new) = (d.join("tool"), d.join("tool.new"));
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("settings.txt"), "eski").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("tool.exe"), "yeni").unwrap();
        swap_dir(&new, &target).unwrap();
        assert!(target.join("tool.exe").exists() && !target.join("settings.txt").exists());
        assert!(!new.exists() && !d.join("tool.old").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(windows)]
    #[test]
    fn zip_update_keeps_old_folder_when_locked() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = temp("swaplock");
        let (target, new) = (d.join("tool"), d.join("tool.new"));
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("tool.exe"), "eski").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("tool.exe"), "yeni").unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(target.join("tool.exe")).unwrap();
        let err = swap_dir(&new, &target).unwrap_err();
        drop(held);
        assert!(err.contains("uygulama açık"), "{err}");
        assert_eq!(std::fs::read_to_string(target.join("tool.exe")).unwrap(), "eski");
        assert!(!new.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(windows)]
    #[test]
    fn exe_update_keeps_old_file_on_failure() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = temp("exe");
        let (target, new) = (d.join("Tool.exe"), d.join("download.exe"));
        std::fs::write(&target, "eski").unwrap();
        std::fs::write(&new, "yeni").unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&target).unwrap();
        let err = replace_exe(&new, &target).unwrap_err();
        drop(held);
        assert!(err.contains("uygulama açık"), "{err}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "eski");
        assert!(!d.join("Tool.exe.new").exists());
        replace_exe(&new, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "yeni");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn inno_exit_codes_are_told_apart() {
        assert!(inno_exit(Some(0)).is_ok());
        let before = inno_exit(Some(2)).unwrap_err();
        let during = inno_exit(Some(5)).unwrap_err();
        assert!(before.contains("başlamadan"), "{before}");
        assert!(during.contains("yarıda"), "{during}");
        assert!(inno_exit(Some(4)).unwrap_err().contains("4"));
        assert!(inno_exit(None).is_err());
    }
}
