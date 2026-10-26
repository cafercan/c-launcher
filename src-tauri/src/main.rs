// Windows'ta release sürümünde arkada konsol penceresi açılmasın
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod detect;
mod github;
mod install;
mod opener;
mod selfupdate;
mod store;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use github::Release;
use launcher_core::{select, status, App, Catalog, Group, Method, Os, Status, Version};
use serde::Serialize;
use store::Store;
use tauri::{AppHandle, Emitter, State};

struct Loaded {
    catalog: Catalog,
    releases: HashMap<String, Result<Option<Release>, String>>,
    launcher_latest: Option<Release>,
    source: &'static str,
    checked_at: u64,
}

struct AppState {
    store: Store,
    http: reqwest::Client,
    loaded: tokio::sync::Mutex<Option<Loaded>>,
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Serialize)]
struct AppView {
    id: String,
    name: String,
    repo: String,
    group: String,
    description: String,
    accent: Option<String>,
    icon: Option<String>,
    usage: Option<String>,
    method: Option<Method>,
    status: Status,
    /// "kurulu ama sürümü okunamadı" durumunda None ve installed_known=true
    installed: Option<String>,
    installed_known: bool,
    latest: Option<String>,
    published_at: Option<String>,
    release_url: Option<String>,
    size: Option<u64>,
    needs_reload: bool,
    busy: bool,
    error: Option<String>,
}

#[derive(Serialize)]
struct Overview {
    groups: Vec<Group>,
    apps: Vec<AppView>,
    os: &'static str,
    source: &'static str,
    checked_at: u64,
    launcher_version: &'static str,
    launcher_latest: Option<String>,
    launcher_url: String,
    brand_name: &'static str,
    home_url: &'static str,
    group_tabs: bool,
}

#[derive(Clone, Serialize)]
struct JobEvent {
    id: String,
    phase: String,
    done: u64,
    total: u64,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn version_of(release: &Release) -> Option<Version> {
    Version::parse(&release.tag_name)
}

async fn fetch(state: &AppState) -> Loaded {
    let cache = state.store.dir("cache");
    let (catalog, source) = github::catalog(&state.http, &cache).await;
    let tasks = catalog.apps.iter().map(|a| {
        let (http, cache, repo, id) = (state.http.clone(), cache.clone(), a.repo.clone(), a.id.clone());
        async move { (id, github::latest_release(&http, &repo, &cache).await) }
    });
    let (releases, launcher) = tokio::join!(futures_util::future::join_all(tasks), github::latest_release(&state.http, &github::BRAND.launcher_repo, &cache));
    Loaded { catalog, releases: releases.into_iter().collect(), launcher_latest: launcher.ok().flatten(), source, checked_at: now() }
}

fn view(app: &App, loaded: &Loaded, state: &AppState) -> AppView {
    let os = Os::current();
    let spec = app.spec_for(os);
    let (release, error) = match loaded.releases.get(&app.id) {
        Some(Ok(r)) => (r.as_ref(), None),
        Some(Err(e)) => (None, Some(e.clone())),
        None => (None, None),
    };
    let installed = spec.and_then(|s| detect::installed(&app.id, s, &state.store));
    let root_set = state.store.install_root().is_some();
    // bu sistem için tarif var mı ve release'te ona uyan dosya var mı
    let asset = spec.zip(release).and_then(|(s, r)| match s.method {
        Method::Script => r.zipball_url.as_ref().map(|_| None),
        _ => {
            let portable = install::uses_portable(s, installed.as_ref(), root_set);
            let names: Vec<&str> = r.assets.iter().map(|a| a.name.as_str()).collect();
            let name = select::pick(install::asset_pattern(s, portable).unwrap_or(""), &names)?;
            Some(r.assets.iter().find(|a| a.name == name).map(|a| a.size))
        }
    });
    let installed_version = installed.as_ref().and_then(|i| i.version.as_deref()).and_then(Version::parse);
    let latest = release.and_then(version_of);
    let supported = spec.is_some() && (release.is_none() || asset.is_some() || installed.is_some());
    let mut st = status::compute(supported, latest.as_ref(), installed_version.as_ref(), installed.is_some());
    // güncelleme var ama bu sistem için yeni dosya yoksa güncelleme önermeyelim
    if st == Status::UpdateAvailable && asset.is_none() {
        st = Status::UpToDate;
    }
    let record = state.store.record(&app.id);
    AppView {
        id: app.id.clone(),
        name: app.name.clone(),
        repo: app.repo.clone(),
        group: app.group.clone(),
        description: app.description.clone(),
        accent: app.accent.clone(),
        icon: app.icon.clone(),
        usage: app.usage.clone(),
        method: spec.map(|s| s.method),
        status: st,
        installed: installed.as_ref().and_then(|i| i.version.clone()),
        installed_known: installed.is_some(),
        latest: latest.map(|v| v.to_string()),
        published_at: release.and_then(|r| r.published_at.clone()),
        release_url: release.map(|r| r.html_url.clone()),
        size: asset.flatten(),
        needs_reload: record.is_some_and(|r| r.needs_reload) && installed.is_some(),
        busy: state.jobs.lock().unwrap().contains_key(&app.id),
        error,
    }
}

fn overview_of(loaded: &Loaded, state: &AppState) -> Overview {
    let launcher_latest = loaded
        .launcher_latest
        .as_ref()
        .and_then(version_of)
        .filter(|v| Version::parse(env!("CARGO_PKG_VERSION")).is_some_and(|cur| *v > cur))
        .map(|v| v.to_string());
    Overview {
        groups: loaded.catalog.groups.clone(),
        apps: loaded.catalog.apps.iter().map(|a| view(a, loaded, state)).collect(),
        os: Os::current().key(),
        source: loaded.source,
        checked_at: loaded.checked_at,
        launcher_version: env!("CARGO_PKG_VERSION"),
        launcher_latest,
        launcher_url: format!("https://github.com/{}/releases/latest", github::BRAND.launcher_repo),
        brand_name: &github::BRAND.name,
        home_url: &github::BRAND.home_url,
        group_tabs: github::BRAND.group_tabs,
    }
}

/// Açılışta ve her işlemden sonra: GitHub'a yeniden sormadan durumu tazeler.
#[tauri::command]
async fn overview(state: State<'_, AppState>) -> Result<Overview, String> {
    let mut guard = state.loaded.lock().await;
    if guard.is_none() {
        *guard = Some(fetch(&state).await);
    }
    Ok(overview_of(guard.as_ref().unwrap(), &state))
}

/// "Yenile" düğmesi: katalog ve release'leri yeniden çeker.
#[tauri::command]
async fn refresh(state: State<'_, AppState>) -> Result<Overview, String> {
    let fresh = fetch(&state).await;
    let mut guard = state.loaded.lock().await;
    *guard = Some(fresh);
    Ok(overview_of(guard.as_ref().unwrap(), &state))
}

async fn find(state: &AppState, id: &str) -> Result<(App, Option<Release>), String> {
    let guard = state.loaded.lock().await;
    let loaded = guard.as_ref().ok_or("liste henüz yüklenmedi")?;
    let app = loaded.catalog.apps.iter().find(|a| a.id == id).cloned().ok_or("uygulama katalogda yok")?;
    let release = loaded.releases.get(id).and_then(|r| r.as_ref().ok()).cloned().flatten();
    Ok((app, release))
}

/// Kurar ya da günceller. İlerleme "job" olayıyla bildirilir.
#[tauri::command]
async fn install(handle: AppHandle, state: State<'_, AppState>, id: String) -> Result<String, String> {
    let (app, release) = find(&state, &id).await?;
    let spec = app.spec_for(Os::current()).cloned().ok_or("bu sistem için sürüm yok")?;
    let release = release.ok_or("henüz release yok")?;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = state.jobs.lock().unwrap();
        if jobs.contains_key(&id) {
            return Err("bu uygulama için zaten bir işlem sürüyor".into());
        }
        jobs.insert(id.clone(), cancel.clone());
    }
    let emit_id = id.clone();
    let h = handle.clone();
    let progress = move |phase: &str, done: u64, total: u64| {
        let _ = h.emit("job", JobEvent { id: emit_id.clone(), phase: phase.into(), done, total });
    };
    let ctx = install::Ctx { http: &state.http, store: &state.store, cancel, progress: &progress };
    let result = install::install(&app, &spec, &release, &ctx).await;
    state.jobs.lock().unwrap().remove(&id);
    let _ = handle.emit("job", JobEvent { id, phase: "done".into(), done: 0, total: 0 });
    result
}

#[tauri::command]
fn cancel(state: State<'_, AppState>, id: String) {
    if let Some(flag) = state.jobs.lock().unwrap().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}

#[derive(Serialize)]
struct Opened {
    /// "launched" | "chrome" | "terminal"
    kind: &'static str,
    path: Option<String>,
    usage: Option<String>,
}

#[tauri::command]
async fn open(state: State<'_, AppState>, id: String) -> Result<Opened, String> {
    let (app, _) = find(&state, &id).await?;
    let spec = app.spec_for(Os::current()).cloned().ok_or("bu sistem için sürüm yok")?;
    match spec.method {
        Method::Chrome => {
            let dir = detect::extension_dir(&state.store, &app.id);
            // kullanıcı Chrome'da yeniledikten sonra uyarıyı kaldır
            if let Some(mut r) = state.store.record(&app.id) {
                r.needs_reload = false;
                state.store.set_record(&app.id, Some(r));
            }
            let _ = opener::open_chrome_extensions();
            Ok(Opened { kind: "chrome", path: Some(dir.display().to_string()), usage: None })
        }
        Method::Script => Ok(Opened { kind: "terminal", path: None, usage: app.usage.clone() }),
        _ => {
            let installed = detect::installed(&app.id, &spec, &state.store).ok_or("uygulama kurulu değil")?;
            opener::open_app(&spec, &installed)?;
            Ok(Opened { kind: "launched", path: None, usage: None })
        }
    }
}

#[tauri::command]
async fn uninstall(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let (app, _) = find(&state, &id).await?;
    let spec = app.spec_for(Os::current()).cloned().ok_or("bu sistem için sürüm yok")?;
    tokio::task::block_in_place(|| install::uninstall(&app, &spec, &state.store))
}

#[derive(Serialize)]
struct Settings {
    install_root: Option<String>,
    suggested_root: String,
    os: &'static str,
}

#[tauri::command]
fn settings(state: State<'_, AppState>) -> Settings {
    Settings {
        install_root: state.store.install_root().map(|p| p.display().to_string()),
        suggested_root: dirs::home_dir().unwrap_or_default().join("Apps").join("CLauncher").display().to_string(),
        os: Os::current().key(),
    }
}

/// Kurulum klasörünü ayarlar; `None` sistemin varsayılan yerlerine döner.
#[tauri::command]
async fn set_install_root(state: State<'_, AppState>, path: Option<String>) -> Result<Overview, String> {
    let path = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    if let Some(p) = &path {
        let dir = std::path::Path::new(p);
        if !dir.is_absolute() {
            return Err("tam bir klasör yolu girin".into());
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("klasör oluşturulamadı: {e}"))?;
        let probe = dir.join(".c-launcher-write-test");
        std::fs::write(&probe, b"").map_err(|_| "bu klasöre yazılamıyor".to_string())?;
        let _ = std::fs::remove_file(probe);
    }
    state.store.set_install_root(path);
    let guard = state.loaded.lock().await;
    let loaded = guard.as_ref().ok_or("liste henüz yüklenmedi")?;
    Ok(overview_of(loaded, &state))
}

/// C Launcher'ın kendisini günceller ve yeniden başlatır. İlerleme "job" olayında
/// "c-launcher" kimliğiyle bildirilir.
#[tauri::command]
async fn update_launcher(handle: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    const ID: &str = "c-launcher";
    let release = {
        let guard = state.loaded.lock().await;
        let loaded = guard.as_ref().ok_or("liste henüz yüklenmedi")?;
        loaded.launcher_latest.clone().ok_or("yeni sürüm bilgisi yok")?
    };
    let current = Version::parse(env!("CARGO_PKG_VERSION")).ok_or("sürüm okunamadı")?;
    if !version_of(&release).is_some_and(|v| v > current) {
        return Err("zaten en son sürüm".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = state.jobs.lock().unwrap();
        if jobs.contains_key(ID) {
            return Err("güncelleme zaten sürüyor".into());
        }
        jobs.insert(ID.into(), cancel.clone());
    }
    let h = handle.clone();
    let progress = move |phase: &str, done: u64, total: u64| {
        let _ = h.emit("job", JobEvent { id: ID.into(), phase: phase.into(), done, total });
    };
    let ctx = install::Ctx { http: &state.http, store: &state.store, cancel, progress: &progress };
    let result = selfupdate::run(&release, &ctx, true).await;
    state.jobs.lock().unwrap().remove(ID);
    let _ = handle.emit("job", JobEvent { id: ID.into(), phase: "done".into(), done: 0, total: 0 });
    result?;
    // yeni sürüm başlatıldı (Windows'ta kurulum bitince açılacak): bu kopya kapanır
    handle.exit(0);
    Ok(())
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    opener::open_url(&url)
}

#[tauri::command]
fn open_chrome_extensions() -> Result<(), String> {
    opener::open_chrome_extensions()
}

/// `c-launcher --self-update`: pencere açmadan en son sürüme günceller (betikler ve denemeler için).
fn cli_self_update() -> i32 {
    let runtime = tokio::runtime::Runtime::new().expect("tokio");
    runtime.block_on(async {
        let store = Store::open();
        let http = github::client();
        let release = match github::latest_release(&http, &github::BRAND.launcher_repo, &store.dir("cache")).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                eprintln!("henüz release yok");
                return 1;
            }
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        };
        let current = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        if !version_of(&release).is_some_and(|v| v > current) {
            println!("{} {current} güncel", github::BRAND.name);
            return 0;
        }
        let progress = |_: &str, _: u64, _: u64| {};
        let ctx = install::Ctx { http: &http, store: &store, cancel: Arc::new(AtomicBool::new(false)), progress: &progress };
        match selfupdate::run(&release, &ctx, false).await {
            Ok(()) => {
                println!("{} {current} → {} güncellendi", github::BRAND.name, release.tag_name);
                0
            }
            Err(e) => {
                eprintln!("güncellenemedi: {e}");
                1
            }
        }
    })
}

fn main() {
    if std::env::args().any(|a| a == "--self-update") {
        std::process::exit(cli_self_update());
    }
    if std::env::args().any(|a| a == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }
    tauri::Builder::default()
        .manage(AppState {
            store: Store::open(),
            http: github::client(),
            loaded: tokio::sync::Mutex::new(None),
            jobs: Mutex::new(HashMap::new()),
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            overview,
            refresh,
            install,
            cancel,
            open,
            uninstall,
            settings,
            set_install_root,
            update_launcher,
            open_url,
            open_chrome_extensions
        ])
        .run(tauri::generate_context!())
        .expect("launcher başlatılamadı");
}
