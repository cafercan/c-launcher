//! GitHub token'ının nereye gönderilebileceği. Token yalnızca API sunucusuna gider; API'nin
//! yönlendirdiği indirme sunucularına, raw ya da CDN adreslerine asla gitmez.

const API: &str = "https://api.github.com/";

/// Bu adrese yapılan isteğe `Authorization` başlığı eklenebilir mi?
pub fn token_allowed(url: &str) -> bool {
    url.starts_with(API)
}

/// Release dosyasının indirileceği adres: girişle okunan depolarda API adresi (token ve
/// `Accept: application/octet-stream` ile), diğerlerinde herkese açık indirme bağlantısı.
pub fn asset_url<'a>(api_url: &'a str, browser_url: &'a str, private: bool) -> &'a str {
    if private && !api_url.is_empty() {
        api_url
    } else {
        browser_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_only_for_api_host() {
        assert!(token_allowed("https://api.github.com/repos/a/b/releases/assets/1"));
        for url in [
            "http://api.github.com/repos/a/b",
            "https://api.github.com.evil.com/",
            "https://api.github.com@evil.com/",
            "https://objects.githubusercontent.com/github-production-release-asset/1",
            "https://raw.githubusercontent.com/a/b/main/catalog.json",
            "https://github.com/a/b/releases/download/v1/x.exe",
        ] {
            assert!(!token_allowed(url), "{url}");
        }
    }

    #[test]
    fn asset_url_uses_api_only_for_private() {
        let (api, browser) = ("https://api.github.com/repos/a/b/releases/assets/1", "https://github.com/a/b/releases/download/v1/x.exe");
        assert_eq!(asset_url(api, browser, true), api);
        assert_eq!(asset_url(api, browser, false), browser);
        assert_eq!(asset_url("", browser, true), browser);
    }
}
