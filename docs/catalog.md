# Uygulama ekleme ve release kuralları

C Launcher'da görünen uygulamalar bu deponun kökündeki [`catalog.json`](../catalog.json)
dosyasından gelir. Launcher her açılışta bu dosyayı GitHub'dan okur; bir uygulama eklemek ya
da çıkarmak için yalnızca bu dosyayı değiştirip `main` dalına göndermek yeterli. Launcher'ın yeni
sürümünü çıkarmak gerekmez. İnternet yoksa son okunan liste, o da yoksa launcher'a gömülü liste
kullanılır.

## Katalog

```jsonc
{
  "schema": 1,
  "groups": [ { "id": "desktop", "title": "Masaüstü uygulamaları" } ],
  "apps": [
    {
      "id": "kitaplik",                 // küçük harf, rakam, tire
      "name": "Kitaplık",
      "repo": "cafercan/kitaplik",      // release'leri okunacak depo
      "group": "desktop",
      "description": "Kısa açıklama",
      "accent": "#2f6f5e",              // kartın rengi
      "icon": "https://raw.githubusercontent.com/...png",  // isteğe bağlı
      "usage": "Terminalde: xd <ad>",   // terminal araçları için isteğe bağlı
      "platforms": { "windows": { … }, "macos": { … }, "linux": { … } }   // ya da her sistem için "any"
    }
  ]
}
```

Bir sistem için `platforms` içinde kayıt yoksa kart o sistemde soluk görünür ve
"Bu sistem için yok" yazar.

## Kurulum yöntemleri

| `method` | Sistem | Gerekenler | Nasıl kurulur |
|---|---|---|---|
| `inno` | Windows | `asset`, `inno_app_id` | Inno Setup kurulumu sessiz kipte çalıştırılır (`/VERYSILENT`). Kurulu sürüm kayıt defterindeki `{AppId}_is1` anahtarından okunur |
| `dmg` | macOS | `asset`, `app` | DMG bağlanır, `.app` `/Applications`'a (yazılamıyorsa `~/Applications`'a) kopyalanır. Sürüm `Info.plist`'teki `CFBundleShortVersionString`'den okunur |
| `deb` | Linux | `asset`, `package`, `run` | `pkexec apt-get install` ile kurulur (sistem parola penceresi açılır). Sürüm `dpkg`'den okunur |
| `script` | hepsi | `install`, isteğe bağlı `uninstall`, `detect_path` / `detect_paths` | Release'in kaynak arşivi indirilir, depodaki betik çalıştırılır (`.ps1` PowerShell ile, diğerleri bash ile) |
| `chrome` | hepsi (`any`) | `asset` | Eklenti zip'i launcher'ın veri klasörüne açılır; kullanıcı Chrome'a bir kez "Paketlenmemiş öğe yükle" ile ekler. Güncellemeler aynı klasöre yazılır |
| `exe` | Windows | `asset`, isteğe bağlı `app` (diskteki dosya adı, varsayılan `<id>.exe`) | Kurulumsuz tek `.exe`, `%LOCALAPPDATA%\Programs\c-launcher\<id>` (ya da seçilen kurulum klasörü) altına konur. Başlat menüsüne kısayol eklenir |
| `zip` | Windows | `asset`, `run` | Kurulumsuz uygulama klasörü. Zip'in kökünde **tek bir klasör** olmalı (adında sürüm olabilir); launcher bu klasörü atarak açar. `run`, o klasörün içinde çalıştırılacak `.exe`'nin yolu. Başlat menüsüne kısayol eklenir |

`asset`, release'teki dosya adı için `*` içerebilen bir kalıptır: `kitaplik-*-windows-x64-setup.exe`.

`exe` ve `zip` yöntemlerinde `SHA256SUMS.txt` zorunludur; yoksa dosya kurulmaz. Güncellemede yeni
dosya ya da klasör önce yanına hazırlanır, sonra eskisinin yerine konur; uygulama açıksa eskisi
yerinde kalır ve "uygulama açık" uyarısı çıkar. Uygulama ayarlarını kendi klasörüne değil
`%APPDATA%` altına yazmalı: güncellemede klasör tümüyle yenilenir.

```json
"windows": { "method": "zip", "asset": "gridcraft-*-windows-x64-portable.zip", "run": "gridcraft.exe" }
```

## Bir depoda birden çok uygulama

Birden çok uygulama release'lerini aynı depoda yayımlıyorsa her biri bir **etiket öneki** taşır:
`"tag_prefix": "alpha-"` ile launcher yalnızca `alpha-v0.1.5` gibi etiketlere bakar ve
sürümü önekten sonraki kısımdan okur. Önek küçük harf, rakam ve tireden oluşur, tireyle biter.
Aynı depoyu paylaşan uygulamaların hepsinin farklı bir öneki olmalı. Seyrek release çıkan bir
uygulama diğerlerinin release'leri altında kalsa da bulunur (en çok 500 release geriye bakılır).

`script` yönteminde araç elle kurulmuş olsa da "kurulu" görünsün diye kurulumun bıraktığı bir
dosya ya da klasör verilir. `detect_path` tek yol, `detect_paths` yol listesidir; biri varsa araç
kurulu sayılır. `~/` ev dizini, `{documents}/` Belgeler klasörüdür (Windows'ta OneDrive'a
yönlendirilmiş olabilir, `~/Documents` yazmak yetmez). `{documents}` yalnızca `detect_paths`'te
kullanılmalı: eski launcher sürümleri `detect_paths`'i yok sayar, `detect_path`'teki yer tutucuyu
ise düz yol sanıp kurulu aracı kurulu değil gösterir. Algılama yolu yoksa yalnızca launcher'ın
kendi kurulum kaydına bakılır.

## Kurulum klasörü

Kullanıcı ayarlardan bir kurulum klasörü seçerse **yeni** kurulumlar oraya yapılır; daha önce
kurulmuş uygulamalar güncellemede bulundukları yerde kalır:

- **Windows (`inno`):** klasör Inno Setup'a `/DIR=` ile verilir (`<klasör>\<Uygulama adı>`).
- **macOS (`dmg`):** `.app` doğrudan seçilen klasöre kopyalanır.
- **Chrome:** eklenti `<klasör>/Chrome eklentileri/<id>` altına açılır.
- **Linux (`deb`):** `.deb` paketlerinin yeri değiştirilemez. Uygulama release'inde taşınabilir
  bir `.tar.gz` sunuyorsa launcher onu `<klasör>/<Uygulama adı>` altına açar ve
  `~/.local/share/applications` altına menü kısayolu yazar (yönetici parolası gerekmez):

```json
"linux": {
  "method": "deb", "asset": "kitaplik-*-linux-all.deb", "package": "kitaplik", "run": "kitaplik",
  "portable": {
    "asset": "kitaplik-*-linux-portable.tar.gz",
    "run": "kitaplik/kitaplik",
    "icon": "kitaplik/kitaplik.svg",
    "wm_class": "kitaplik"
  }
}
```

  `run` ve `icon` arşivin içindeki yollardır; `wm_class`, pencerenin menüdeki kısayolla
  eşleşmesi için uygulamanın pencere sınıfıdır. Taşınabilir paketi olmayan uygulamalar Linux'ta
  her zaman `.deb` olarak kurulur.
- **Terminal araçları:** yeri kurulum betiği belirler.

## Kendi derlemeniz (brand.json)

Launcher'ın adı, veri klasörü, katalogları ve güncelleme deposu derlemeye gömülen
[`src-tauri/brand.json`](../src-tauri/brand.json) dosyasından gelir. Kodu çatallamadan başka
adla ayrı bir launcher derlemek için bu dosyayı, simgeleri ve bir Tauri ayar katmanını
değiştirmek yeterli:

```jsonc
{
  "name": "Örnek Launcher",
  "data_dir": "ornek-launcher",                  // %LOCALAPPDATA% altındaki klasör
  "launcher_repo": "example-org/releases",       // launcher'ın kendi güncellemeleri
  "launcher_tag_prefix": "launcher-",            // depo araçlarla paylaşılıyorsa
  "home_url": "https://github.com/example-org",
  "login": { "client_id": "Iv1.0123456789abcdef" },  // isteğe bağlı GitHub girişi
  "group_tabs": true,                            // gruplar sekme olarak görünür
  "sources": [
    { "repo": "example-org/releases", "private": true,
      "mirror": "http://192.0.2.10/releases" },          // isteğe bağlı yerel ayna
    { "repo": "cafercan/c-launcher", "embedded": true,
      "group": { "id": "public", "title": "Herkese açık uygulamalar" } }
  ]
}
```

- **`sources`:** kataloglar sırayla okunup birleştirilir; aynı kimlik iki kaynakta varsa öndeki
  kazanır. `group` verilen bir kaynağın bütün uygulamaları o grupta görünür. Her kaynağın
  ayrı önbelleği vardır; çevrimdışıyken her biri kendi son kopyasına düşer. `embedded` yalnızca
  launcher'a gömülü `catalog.json`'ın aynısı olan kaynak içindir.
- **`private` ve `login`:** özel bir kaynak giriş yapılana kadar okunmaz; yerinde bir giriş paneli
  görünür, diğer kaynaklar girişsiz çalışır. Giriş, GitHub'ın cihaz koduyla giriş akışını kullanır:
  bir **GitHub App** oluşturun (OAuth App değil), "Enable Device Flow"u açın, izin olarak yalnızca
  *Contents: read* ve *Metadata: read* verin ve uygulamayı yalnızca katalog ve release depolarına
  kurun. Token Windows Kimlik Bilgileri Yöneticisi'nde / macOS Anahtar Zinciri'nde tutulur ve
  yalnızca `api.github.com`'a gönderilir; süresi dolunca ya da geri alınınca launcher yeniden giriş
  ister. GitHub App'lerin kullanıcı token'ları varsayılan olarak 8 saatte dolar ve launcher yenileme
  token'ı kullanmaz: kullanıcıların her gün yeniden giriş yapmaması için uygulamanın ayarlarında
  "User-to-server token expiration"ı kapatın.
- **`mirror`:** yerel ağdaki bir web sunucusu. Launcher açılışta bu adresi en çok 2 saniye
  dener; ulaşırsa kataloğu (`<mirror>/catalog.json`) ve release listesini (`<mirror>/releases.json`,
  GitHub'ın `/releases` yanıtıyla aynı biçimde, indirme adresleri aynayı gösterir) girişsiz oradan
  okur. Ulaşamazsa kaynak her zamanki gibi GitHub'dan (özelse girişle) okunur. Dosyalar aynada da
  `SHA256SUMS.txt` ile doğrulanır. Aynanın içeriğini GitHub'dan düzenli olarak indirmek kurulumun
  işidir; launcher yalnızca okur.
- **Derleme:** `src-tauri/brand.json` ve `src-tauri/icons/` değiştirilip
  `npx @tauri-apps/cli@2 build --config overlay.json` çalıştırılır; `overlay.json` en az
  `productName` ve `identifier` (ör. `com.example.launcher`) içermeli ki iki launcher yan yana
  kurulabilsin.

## Uyumlu release için kurallar

1. **Etiket sürümdür.** `v1.2.0` gibi; launcher bunu kurulu sürümle karşılaştırır. Uygulamanın
   kendi bildirdiği sürüm (Inno `AppVersion`, macOS `CFBundleShortVersionString`, `.deb` `Version`)
   etiketle aynı olmalı, yoksa güncelleme hep "var" görünür.
2. **Kimlik hiç değişmez.** Inno `AppId`, macOS paket kimliği, `.deb` paket adı ilk sürümden
   sonra değişmemeli. Güncelleme aynı kimlikle aynı yere kurulduğu için kullanıcının "varsayılan
   uygulama" ve dosya ilişkilendirme ayarları korunur.
3. **`SHA256SUMS.txt` ekleyin.** `sha256sum * > SHA256SUMS.txt` çıktısı. Varsa launcher her
   dosyayı kurmadan önce doğrular, tutmazsa kurmaz.
4. **Dosya adları tutarlı olsun.** Her sürümde aynı kalıba uysun (yalnızca sürüm numarası değişsin).
5. **Windows kurulumu sessiz kipte çalışabilmeli.** Inno Setup'ta `CloseApplications=yes` açık
   uygulamayı güvenle kapatır. Kullanıcı başına kurulum (`PrivilegesRequired=lowest`) yönetici izni istemez.

Örnek bir uygulama için [cafercan/kitaplik](https://github.com/cafercan/kitaplik) deposundaki
`.github/workflows/release.yml` dosyasına bakın: üç sistem için paketleri derler, her paketi
dener ve release'e `SHA256SUMS.txt` ile birlikte yükler.
