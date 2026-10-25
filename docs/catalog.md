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
| `script` | hepsi | `install`, isteğe bağlı `uninstall`, `detect_path` | Release'in kaynak arşivi indirilir, depodaki betik çalıştırılır (`.ps1` PowerShell ile, diğerleri bash ile) |
| `chrome` | hepsi (`any`) | `asset` | Eklenti zip'i launcher'ın veri klasörüne açılır; kullanıcı Chrome'a bir kez "Paketlenmemiş öğe yükle" ile ekler. Güncellemeler aynı klasöre yazılır |

`asset`, release'teki dosya adı için `*` içerebilen bir kalıptır: `kitaplik-*-windows-x64-setup.exe`.

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
