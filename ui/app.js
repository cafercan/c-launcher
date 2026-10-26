'use strict';

// ---------------------------------------------------------------- arka planla konuşma
// Tauri dışında (tarayıcıda) açılırsa örnek verilerle çalışır: arayüzü denemek için.
const TAURI = window.__TAURI__;
let mockListener = null;
const invoke = TAURI ? TAURI.core.invoke : (cmd, args) => mock(cmd, args);
const listen = TAURI ? TAURI.event.listen : (_, cb) => { mockListener = cb; };

const $ = s => document.querySelector(s);
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

const STATUS = {
  update_available: 'GÜNCELLEME VAR',
  not_installed: 'KURULABİLİR',
  up_to_date: 'KURULU · GÜNCEL',
  unsupported: 'BU SİSTEM İÇİN YOK',
  no_release: 'HENÜZ SÜRÜM YOK',
};
const OS_NAME = { windows: 'WINDOWS', macos: 'MACOS', linux: 'LINUX' };
const PHASE = { download: 'İndiriliyor', verify: 'Doğrulanıyor', install: 'Kuruluyor' };
const SOURCE = { github: 'GITHUB', cache: 'ÖNBELLEK (ÇEVRİMDIŞI)', embedded: 'GÖMÜLÜ LİSTE', none: 'LİSTE ALINAMADI' };
// alt çubukta en kötü durumdaki kaynak görünür; giriş bekleyen kaynak hata sayılmaz
const worstSource = () => ['none', 'embedded', 'cache'].find(f => data.sources.some(s => s.from === f)) || 'github';

let data = null;
let view = localStorage.getItem('view') || 'grid';
let filter = 'all';
let query = '';
let tab = localStorage.getItem('group') || 'all';
const jobs = {};      // id -> {phase, done, total}
const errors = {};    // id -> son hata

// ---------------------------------------------------------------- yardımcılar
const mb = b => (b / 1048576).toFixed(b > 104857600 ? 0 : 1) + ' MB';
const date = iso => iso ? new Date(iso).toLocaleDateString('tr-TR', { day: 'numeric', month: 'short', year: 'numeric' }) : '';
function ago(sec) {
  const m = Math.round((Date.now() / 1000 - sec) / 60);
  return m < 1 ? 'AZ ÖNCE' : m < 60 ? `${m} DK ÖNCE` : `${Math.round(m / 60)} SA ÖNCE`;
}
function toast(msg, bad = false) {
  const t = $('#toast');
  t.textContent = msg;
  t.className = 'toast' + (bad ? ' bad' : '');
  t.hidden = false;
  clearTimeout(toast.t);
  toast.t = setTimeout(() => (t.hidden = true), bad ? 8000 : 5000);
}

function statusOf(app) {
  if (jobs[app.id]) return 'busy';
  if (app.needs_reload) return 'reload';
  return app.status;
}

function visible(app) {
  if (query && !(app.name + ' ' + app.description + ' ' + app.repo).toLowerCase().includes(query)) return false;
  if (filter === 'installed') return app.installed_known;
  if (filter === 'updates') return app.status === 'update_available';
  if (filter === 'missing') return !app.installed_known && app.status !== 'unsupported';
  return true;
}

// ---------------------------------------------------------------- çizim
function icon(app) {
  const bg = app.accent || '#555';
  const letter = esc((app.name[0] || '?').toLocaleUpperCase('tr'));
  const img = app.icon ? `<img src="${esc(app.icon)}" alt="">` : '';
  return `<div class="app-icon" data-letter="${letter}" style="background:${esc(bg)}">${img || letter}</div>`;
}

function badge(app) {
  const s = statusOf(app);
  const label = s === 'busy' ? (PHASE[jobs[app.id].phase] || 'Çalışıyor').toUpperCase() + '…'
    : s === 'reload' ? "CHROME'DA YENİLE" : STATUS[s];
  return `<span class="badge s-${s}">${label}</span>`;
}

function versionLine(app) {
  const when = app.published_at ? ` · ${date(app.published_at)}` : '';
  if (app.status === 'update_available') return `<b>${esc(app.installed)}</b> kurulu → <b>${esc(app.latest)}</b> yayında${when}`;
  if (app.installed_known && app.installed) return `Sürüm <b>${esc(app.installed)}</b>${when}`;
  if (app.installed_known) return `Kurulu${app.latest ? ` · son sürüm ${esc(app.latest)}` : ''}`;
  if (app.status === 'unsupported') return `${OS_NAME[data.os] || 'Bu sistem'} için henüz paket yok`;
  if (app.latest) return `Sürüm <b>${esc(app.latest)}</b>${when}${app.size ? ` · ${mb(app.size)}` : ''}`;
  return 'GitHub\'da henüz release yok';
}

function primary(app) {
  const s = app.status;
  const open = app.method === 'chrome' ? "CHROME'DA AÇ" : app.method === 'script' ? 'KULLANIM' : 'AÇ';
  if (s === 'update_available') return `<button class="btn" data-act="install" data-id="${app.id}">${esc(app.latest)} SÜRÜMÜNE GÜNCELLE</button>`;
  if (s === 'not_installed') return `<button class="btn" data-act="install" data-id="${app.id}">KUR</button>`;
  if (s === 'up_to_date') return `<button class="btn ghost" data-act="open" data-id="${app.id}">${app.needs_reload ? "CHROME'DA YENİLE" : open}</button>`;
  return `<button class="btn ghost" disabled>${s === 'no_release' ? 'YAKINDA' : 'KULLANILAMAZ'}</button>`;
}

function actions(app) {
  const job = jobs[app.id];
  if (job) {
    const pct = job.total ? Math.round(job.done / job.total * 100) : 0;
    const text = job.phase === 'download' && job.total
      ? `${PHASE.download} ${mb(job.done)} / ${mb(job.total)} · %${pct}` : (PHASE[job.phase] || 'Çalışıyor') + '…';
    const indet = job.phase !== 'download' || !job.total;
    const cancel = job.phase === 'download' ? `<button class="more" data-act="cancel" data-id="${app.id}" title="İptal">✕</button>` : '';
    return `<div class="actions"><div class="progress">${text}<div class="track ${indet ? 'indet' : ''}"><i style="width:${indet ? '' : pct + '%'}"></i></div></div>${cancel}</div>`;
  }
  return `<div class="actions">${primary(app)}<button class="more" data-act="menu" data-id="${app.id}" title="Diğer">…</button></div>`;
}

function card(app, n) {
  const dim = app.status === 'unsupported' ? ' dim' : '';
  const err = errors[app.id] ? `<div class="err">${esc(errors[app.id])}</div>` : '';
  return `<article class="card${dim}">
    <div class="card-top"><span class="num" style="background:${esc(app.accent || '#555')}">${String(n).padStart(2, '0')}</span>
      <span class="links"><a href="#" data-url="https://github.com/${esc(app.repo)}">GITHUB ↗</a></span></div>
    <div class="card-body">
      <div class="ident">${icon(app)}<div><h3>${esc(app.name)}</h3><p>${esc(app.description)}</p></div></div>
      <div class="meta">${badge(app)}<div class="ver">${versionLine(app)}</div>${actions(app)}${err}</div>
    </div></article>`;
}

function row(app) {
  const dim = app.status === 'unsupported' ? ' class="dim"' : '';
  return `<tr${dim}><td><div class="ident">${icon(app)}<div><h3>${esc(app.name)}</h3><p>${esc(app.description)}</p></div></div></td>
    <td>${badge(app)}</td><td class="mono">${esc(app.installed || (app.installed_known ? '?' : '—'))}</td>
    <td class="mono" style="color:${app.status === 'update_available' ? 'var(--amber)' : ''}">${esc(app.latest || '—')}</td>
    <td>${date(app.published_at)}</td><td class="act">${actions(app)}${errors[app.id] ? `<div class="err">${esc(errors[app.id])}</div>` : ''}</td></tr>`;
}

function render() {
  if (!data) return;
  const apps = data.apps;
  const counts = {
    all: apps.length,
    installed: apps.filter(a => a.installed_known).length,
    updates: apps.filter(a => a.status === 'update_available').length,
    missing: apps.filter(a => !a.installed_known && a.status !== 'unsupported').length,
  };
  const labels = { all: 'TÜMÜ', installed: 'KURULU', updates: 'GÜNCELLEMELER', missing: 'KURULU DEĞİL' };
  $('#filters').innerHTML = Object.keys(labels).map(k =>
    `<button data-filter="${k}" class="${filter === k ? 'on' : ''}">${labels[k]}<span class="n ${k === 'updates' && counts.updates ? 'hot' : ''}">${counts[k]}</span></button>`).join('');
  const pending = apps.filter(a => a.status === 'update_available' && !jobs[a.id]).length;
  $('#updateAll').textContent = pending ? `TÜMÜNÜ GÜNCELLE (${pending})` : 'TÜMÜ GÜNCEL';
  $('#updateAll').disabled = !pending;
  document.querySelectorAll('.views button').forEach(b => b.classList.toggle('on', b.dataset.view === view));

  renderTabs();
  const inTab = id => !data.group_tabs || tab === 'all' || tab === id;
  let n = 0, html = '';
  // girişle okunan kaynaklar: giriş yapılana kadar kart yerine giriş paneli
  data.sources.filter(s => s.from === 'login' && inTab(loginTabId(s))).forEach(s => {
    html += `<div class="section-head"><span>${esc(s.group ? s.group.title : 'Giriş gerektiren uygulamalar')}</span></div>
      <div class="login-panel"><span>Bu bölüm için GitHub hesabınızla giriş yapın.</span><button class="btn" data-act="login">GITHUB İLE GİRİŞ YAP</button></div>`;
  });
  // listesi hiç alınamayan (çevrimdışı ve önbelleksiz) kaynaklar sessizce kaybolmasın
  failedSources().filter(s => inTab(loginTabId(s))).forEach(s => {
    html += `<div class="section-head"><span>${esc(s.group ? s.group.title : s.repo)}</span></div>
      <div class="login-panel"><span>Bu bölümün listesi alınamadı (çevrimdışı olabilirsiniz). Yenile'ye basarak tekrar deneyin.</span>${s.error ? `<span class="err">${esc(s.error)}</span>` : ''}</div>`;
  });
  data.groups.forEach((g, gi) => {
    const list = apps.filter(a => a.group === g.id);
    const shown = list.filter(visible);
    if (!inTab(g.id)) { n += list.length; return; }
    if (!shown.length) { n += list.length; return; }
    html += `<div class="section-head"><span>${String(gi + 1).padStart(2, '0')} / ${esc(g.title)}</span><span>${list.length} UYGULAMA</span></div>`;
    if (view === 'grid') {
      html += '<div class="grid">' + list.map(a => { n++; return visible(a) ? card(a, n) : ''; }).join('') + '</div>';
    } else {
      html += '<table><thead><tr><th>Uygulama</th><th>Durum</th><th>Kurulu</th><th>Son</th><th>Yayın</th><th></th></tr></thead><tbody>'
        + shown.map(row).join('') + '</tbody></table>';
      n += list.length;
    }
  });
  $('#content').innerHTML = html || '<p class="empty">Eşleşen uygulama yok</p>';

  $('#checked').textContent = `SON KONTROL ${ago(data.checked_at)} · ${SOURCE[worstSource()]}`;
  $('#target').textContent = `${data.brand_name.toLocaleUpperCase('tr')} ${data.launcher_version} · KURULUM HEDEFİ: ${OS_NAME[data.os] || data.os}`;
  renderBrand();
  renderUser();
  renderBanner();
}

const loginTabId = s => s.group ? s.group.id : 'src:' + s.repo;
const failedSources = () => data.sources.filter(s => s.from === 'none');

// brand.json'da group_tabs açıksa gruplar sekme olur; seçim hatırlanır
function renderTabs() {
  const nav = $('#tabs');
  nav.hidden = !data.group_tabs;
  if (!data.group_tabs) return;
  const tabs = [['all', 'TÜMÜ', data.apps.length]];
  data.sources.filter(s => s.from === 'login').forEach(s => tabs.push([loginTabId(s), s.group ? s.group.title : 'Giriş gerektiren uygulamalar', null]));
  failedSources().forEach(s => tabs.push([loginTabId(s), s.group ? s.group.title : s.repo, null]));
  data.groups.forEach(g => tabs.push([g.id, g.title, data.apps.filter(a => a.group === g.id).length]));
  if (!tabs.some(t => t[0] === tab)) tab = 'all';
  nav.innerHTML = tabs.map(([id, title, count]) =>
    `<button data-tab="${esc(id)}" class="${tab === id ? 'on' : ''}">${esc(title.toLocaleUpperCase('tr'))}${count === null ? '' : `<span class="n">${count}</span>`}</button>`).join('');
}

function renderUser() {
  const u = $('#user');
  u.hidden = !data.user;
  u.innerHTML = data.user ? `@${esc(data.user)} · <a href="#" data-act="logout">ÇIKIŞ</a>` : '';
}

async function login() {
  try {
    const code = await invoke('login_start');
    $('#userCode').textContent = code.user_code;
    $('#openDevice').dataset.url = code.verification_uri;
    $('#loginState').textContent = 'Onay bekleniyor…';
    $('#login').showModal();
  } catch (e) {
    toast('Giriş başlatılamadı: ' + e, true);
    return;
  }
  try {
    const name = await invoke('login_wait');
    $('#login').close();
    toast(`@${name} olarak giriş yapıldı`);
    load(true);
  } catch (e) {
    $('#loginState').textContent = String(e);
  }
}

async function logout() {
  await invoke('logout');
  load(true);
}

function renderBrand() {
  const name = data.brand_name;
  document.title = name;
  $('#brand').innerHTML = `<span class="logo">${esc((name[0] || '?').toLocaleUpperCase('tr'))}</span>${esc(name.toLocaleUpperCase('tr'))}`;
  $('#home').dataset.url = data.home_url;
}

function renderBanner() {
  const banner = $('#banner');
  banner.hidden = !data.launcher_latest;
  if (!data.launcher_latest) return;
  const job = jobs['c-launcher'];
  if (job) {
    const text = job.phase === 'download' && job.total
      ? `İndiriliyor ${mb(job.done)} / ${mb(job.total)} · %${Math.round(job.done / job.total * 100)}`
      : (PHASE[job.phase] || 'Hazırlanıyor') + '…';
    banner.innerHTML = `${esc(data.brand_name)} ${esc(data.launcher_latest)} güncelleniyor: ${text}`;
    return;
  }
  const err = errors['c-launcher'] ? ` <span class="err">${esc(errors['c-launcher'])}</span>` : '';
  banner.innerHTML = `<span>${esc(data.brand_name)} ${esc(data.launcher_latest)} yayında.</span>
    <button class="btn" id="selfUpdate">GÜNCELLE VE YENİDEN BAŞLAT</button>
    <a href="#" data-url="${esc(data.launcher_url)}">Sürüm notları ↗</a>${err}`;
}

async function updateLauncher() {
  delete errors['c-launcher'];
  jobs['c-launcher'] = { phase: 'download', done: 0, total: 0 };
  renderBanner();
  try {
    await invoke('update_launcher'); // başarıda uygulama kapanır, yeni sürüm açılır
  } catch (e) {
    errors['c-launcher'] = String(e);
  }
  delete jobs['c-launcher'];
  renderBanner();
}

// ---------------------------------------------------------------- işlemler
async function load(fresh = false) {
  $('#refresh').classList.add('spin');
  try {
    data = await invoke(fresh ? 'refresh' : 'overview');
  } catch (e) {
    toast('Liste alınamadı: ' + e, true);
  }
  $('#refresh').classList.remove('spin');
  render();
}

async function install(id) {
  delete errors[id];
  jobs[id] = { phase: 'download', done: 0, total: 0 };
  render();
  try {
    const note = await invoke('install', { id });
    toast(`${appName(id)}: ${note}`);
  } catch (e) {
    if (String(e) !== 'iptal edildi') errors[id] = String(e);
  }
  delete jobs[id];
  await load();
}

async function open(id) {
  try {
    const r = await invoke('open', { id });
    if (r.kind === 'chrome') {
      $('#chromePath').textContent = r.path;
      $('#chrome').showModal();
    } else if (r.kind === 'terminal') {
      toast(r.usage || 'Yeni bir terminal açıp aracı kullanabilirsiniz');
    }
  } catch (e) {
    toast('Açılamadı: ' + e, true);
  }
  load();
}

async function uninstall(id) {
  if (!confirm(`${appName(id)} kaldırılsın mı?`)) return;
  try {
    toast(`${appName(id)}: ${await invoke('uninstall', { id })}`);
  } catch (e) {
    toast('Kaldırılamadı: ' + e, true);
  }
  load();
}

const appName = id => (data.apps.find(a => a.id === id) || {}).name || id;

function showMenu(id, anchor) {
  const app = data.apps.find(a => a.id === id);
  const items = [[`GITHUB'DA GÖRÜNTÜLE`, () => invoke('open_url', { url: `https://github.com/${app.repo}` })]];
  if (app.release_url) items.push(['SÜRÜM NOTLARI', () => invoke('open_url', { url: app.release_url })]);
  if (app.status === 'up_to_date' && app.method === 'chrome') items.push(['KLASÖRÜ GÖSTER', () => open(id)]);
  if (app.installed_known && app.status !== 'unsupported') items.push(['YENİDEN KUR', () => install(id)]);
  if (app.installed_known) items.push(['KALDIR', () => uninstall(id), 'danger']);
  const menu = $('#menu');
  menu.innerHTML = items.map(([label, , cls], i) => `<button data-i="${i}" class="${cls || ''}">${label}</button>`).join('');
  menu.onclick = e => { const b = e.target.closest('button'); if (b) { menu.hidden = true; items[b.dataset.i][1](); } };
  const r = anchor.getBoundingClientRect();
  menu.hidden = false;
  menu.style.top = Math.min(r.bottom + 4, innerHeight - menu.offsetHeight - 8) + 'px';
  menu.style.left = Math.max(8, r.right - menu.offsetWidth) + 'px';
}

// ---------------------------------------------------------------- olaylar
document.addEventListener('click', e => {
  const link = e.target.closest('[data-url]');
  if (link) { e.preventDefault(); invoke('open_url', { url: link.dataset.url }).catch(err => toast(String(err), true)); return; }
  if (!e.target.closest('#menu')) $('#menu').hidden = true;
  const b = e.target.closest('[data-act]');
  if (b) {
    const { act, id } = b.dataset;
    if (act === 'login') { e.preventDefault(); login(); }
    else if (act === 'logout') { e.preventDefault(); logout(); }
    else if (act === 'install') install(id);
    else if (act === 'open') open(id);
    else if (act === 'cancel') invoke('cancel', { id });
    else if (act === 'menu') { e.stopPropagation(); showMenu(id, b); }
    return;
  }
  if (e.target.closest('#selfUpdate')) { updateLauncher(); return; }
  const t = e.target.closest('[data-tab]');
  if (t) { tab = t.dataset.tab; try { localStorage.setItem('group', tab); } catch {} render(); return; }
  const f = e.target.closest('[data-filter]');
  if (f) { filter = f.dataset.filter; render(); }
  const v = e.target.closest('[data-view]');
  if (v) { view = v.dataset.view; try { localStorage.setItem('view', view); } catch {} render(); }
});
// simge yüklenemezse baş harf görünsün (satır içi onerror CSP'ye takılır)
document.addEventListener('error', e => {
  if (e.target.tagName === 'IMG' && e.target.parentElement.classList.contains('app-icon')) {
    const box = e.target.parentElement;
    e.target.remove();
    box.textContent = box.dataset.letter || '?';
  }
}, true);
$('#search').addEventListener('input', e => { query = e.target.value.trim().toLowerCase(); render(); });
$('#refresh').addEventListener('click', () => load(true));
$('#updateAll').addEventListener('click', async () => {
  for (const a of data.apps.filter(a => a.status === 'update_available' && !jobs[a.id])) await install(a.id);
});
$('#closeChrome').addEventListener('click', () => $('#chrome').close());
$('#closeLogin').addEventListener('click', () => $('#login').close());
$('#copyCode').addEventListener('click', async () => {
  try { await navigator.clipboard.writeText($('#userCode').textContent); toast('Kod kopyalandı'); }
  catch { toast('Kopyalanamadı; kodu elle yazın', true); }
});

// ---------------------------------------------------------------- tema
function markTheme() {
  const t = window.themeChoice();
  document.querySelectorAll('[data-theme-set]').forEach(b => b.classList.toggle('on', b.dataset.themeSet === t));
}
$('#themeToggle').addEventListener('click', () => {
  // tek tıkla, şu an görünenin tersine geç
  const dark = getComputedStyle(document.documentElement).colorScheme.includes('dark');
  window.setTheme(dark ? 'light' : 'dark');
  markTheme();
});
$('#themeSeg').addEventListener('click', e => {
  const b = e.target.closest('[data-theme-set]');
  if (b) { window.setTheme(b.dataset.themeSet); markTheme(); }
});

// ---------------------------------------------------------------- ayarlar
const ROOT_NOTE = {
  windows: 'Yeni kurulumlar bu klasöre yapılır. Daha önce kurulmuş uygulamalar güncellemede bulundukları yerde kalır. Chrome eklentileri "Chrome eklentileri" alt klasörüne açılır.',
  macos: 'Yeni kurulumlar bu klasöre kopyalanır. Daha önce kurulmuş uygulamalar güncellemede bulundukları yerde kalır. Chrome eklentileri "Chrome eklentileri" alt klasörüne açılır.',
  linux: 'Yeni kurulumlarda, uygulama sunuyorsa .deb yerine taşınabilir paket bu klasöre açılır ve menüye kısayol eklenir; yönetici parolası gerekmez. Taşınabilir paketi olmayan uygulamalar .deb olarak kurulur. Güncellemeler bulundukları yerde kalır.',
};
async function showSettings() {
  const s = await invoke('settings');
  showSettings.s = s;
  $('#rootPath').textContent = s.install_root || 'Sistem varsayılanı';
  $('#rootNote').textContent = s.install_root ? ROOT_NOTE[s.os] || ''
    : 'Uygulamalar sistemin olağan yerine kurulur (Windows: Programlar, macOS: Uygulamalar, Linux: .deb paketi). Klasör seçerseniz yeni kurulumlar oraya yapılır.';
  $('#resetRoot').disabled = !s.install_root;
  markTheme();
  $('#settings').showModal();
}
async function setRoot(path) {
  try {
    data = await invoke('set_install_root', { path });
    render();
    await showSettings();
  } catch (e) {
    toast('Klasör ayarlanamadı: ' + e, true);
  }
}
$('#openSettings').addEventListener('click', () => showSettings().catch(e => toast(String(e), true)));
$('#closeSettings').addEventListener('click', () => $('#settings').close());
$('#resetRoot').addEventListener('click', () => setRoot(null));
$('#pickRoot').addEventListener('click', async () => {
  const s = showSettings.s || {};
  let path = null;
  if (TAURI && TAURI.dialog) {
    path = await TAURI.dialog.open({ directory: true, title: 'Kurulum klasörünü seçin', defaultPath: s.install_root || s.suggested_root });
  } else {
    path = prompt('Kurulum klasörü', s.install_root || s.suggested_root);
  }
  if (path) setRoot(Array.isArray(path) ? path[0] : path);
});
$('#openExt').addEventListener('click', () => invoke('open_chrome_extensions').catch(e => toast(String(e), true)));
$('#copyPath').addEventListener('click', async () => {
  try { await navigator.clipboard.writeText($('#chromePath').textContent); toast('Klasör yolu kopyalandı'); }
  catch { toast('Kopyalanamadı; yolu elle seçin', true); }
});
listen('job', ev => {
  const { id, phase, done, total } = ev.payload;
  if (phase === 'done') return;
  jobs[id] = { phase, done, total };
  if (id === 'c-launcher') renderBanner(); else render();
});
setInterval(() => data && ($('#checked').textContent = $('#checked').textContent.replace(/^SON KONTROL .*? ·/, `SON KONTROL ${ago(data.checked_at)} ·`)), 30000);

load();

// ---------------------------------------------------------------- örnek veri (yalnızca tarayıcıda)
function mock(cmd, args) {
  const now = Date.now() / 1000;
  const base = {
    groups: [{ id: 'desktop', title: 'Masaüstü uygulamaları' }, { id: 'terminal', title: 'Terminal araçları' }, { id: 'chrome', title: 'Chrome eklentileri' }],
    os: 'linux', sources: [{ repo: 'cafercan/c-launcher', group: null, from: 'github', error: null }], checked_at: now - 180, launcher_version: '0.1.0', launcher_latest: null, launcher_url: '',
    brand_name: 'C Launcher', home_url: 'https://github.com/cafercan', group_tabs: new URLSearchParams(location.search).has('tabs'), login_enabled: false, user: null,
    apps: [
      { id: 'kitaplik', name: 'Kitaplık', repo: 'cafercan/kitaplik', group: 'desktop', description: 'E-kitap kitaplığı: etiket, filtre, kalite puanı', accent: '#2f6f5e', icon: 'https://raw.githubusercontent.com/cafercan/kitaplik/main/kitaplik.svg', method: 'deb', status: 'update_available', installed: '0.1.0', installed_known: true, latest: '0.2.0', published_at: '2026-10-25T10:00:00Z', size: 31000, needs_reload: false },
      { id: 'mdviewer', name: 'MD Flow Viewer', repo: 'cafercan/MDViewer', group: 'desktop', description: 'Markdown görüntüleyici ve düzenleyici: canlı yenileme, Mermaid', accent: '#3b6fd4', icon: 'https://raw.githubusercontent.com/cafercan/MDViewer/main/public/favicon-192.png', method: 'deb', status: 'not_installed', installed: null, installed_known: false, latest: '2.9.0', published_at: '2026-10-12T10:00:00Z', size: 48200000, needs_reload: false },
      { id: 'xd', name: 'xd', repo: 'cafercan/xd', group: 'terminal', description: 'Sık kullanılan klasörlere kısa adlarla gitme', accent: '#b8862b', usage: 'Terminalde: xd -add <ad> <klasör>', method: 'script', status: 'up_to_date', installed: '0.2.0', installed_known: true, latest: '0.2.0', published_at: '2026-09-02T10:00:00Z', needs_reload: false },
      { id: 'anlik-dublaj', name: 'Anlık Dublaj', repo: 'cafercan/anlik-dublaj', group: 'chrome', description: 'Tarayıcıdaki videoları gerçek zamanlı Türkçe dinletir', accent: '#c0392b', icon: 'https://raw.githubusercontent.com/cafercan/anlik-dublaj/main/icons/icon128.png', method: 'chrome', status: 'not_installed', installed: null, installed_known: false, latest: '1.2.0', published_at: '2026-10-01T10:00:00Z', size: 240000, needs_reload: false },
      { id: 'kedy', name: 'Kedy', repo: 'cafercan/kedy', group: 'chrome', description: 'İmleci kafası ve gözleriyle takip eden sevimli kedi', accent: '#8e44ad', icon: 'https://raw.githubusercontent.com/cafercan/kedy/main/icons/icon128.png', method: 'chrome', status: 'up_to_date', installed: '0.1.4', installed_known: true, latest: '0.1.4', published_at: '2026-08-20T10:00:00Z', needs_reload: true },
    ],
  };
  mock.state = mock.state || base;
  if (cmd === 'overview' || cmd === 'refresh') return Promise.resolve(JSON.parse(JSON.stringify(mock.state)));
  if (cmd === 'install') {
    return new Promise(res => {
      let done = 0; const total = 30e6;
      const t = setInterval(() => {
        done += 3e6;
        mockListener && mockListener({ payload: { id: args.id, phase: done < total ? 'download' : 'install', done, total } });
        if (done >= total + 6e6) {
          clearInterval(t);
          const a = mock.state.apps.find(x => x.id === args.id);
          Object.assign(a, { status: 'up_to_date', installed: a.latest, installed_known: true });
          res('Kuruldu');
        }
      }, 250);
    });
  }
  if (cmd === 'settings') return Promise.resolve({ install_root: mock.root || null, suggested_root: '/home/kullanici/Apps/CLauncher', os: 'linux' });
  if (cmd === 'set_install_root') { mock.root = args.path; return Promise.resolve(JSON.parse(JSON.stringify(mock.state))); }
  if (cmd === 'open') return Promise.resolve({ kind: args.id === 'kedy' ? 'chrome' : 'launched', path: '/home/kullanici/.local/share/c-launcher/extensions/kedy' });
  return Promise.resolve('tamam');
}
