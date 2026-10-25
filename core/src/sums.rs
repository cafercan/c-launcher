//! Release'teki `SHA256SUMS.txt` dosyası (`sha256sum` çıktısı biçiminde).

use std::collections::HashMap;

/// Dosya adı → küçük harfli onaltılık özet.
pub fn parse(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((hash, name)) = line.split_once(char::is_whitespace) else { continue };
        // "*ad" ikili kip işaretidir
        let name = name.trim_start().trim_start_matches('*');
        if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) && !name.is_empty() {
            out.insert(name.to_string(), hash.to_ascii_lowercase());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sha256sum_output() {
        let h = "a".repeat(64);
        let text = format!("{h}  kitaplik-0.1.0-linux-all.deb\n{}  *Setup.exe\nbozuk satır\n", "B".repeat(64));
        let m = parse(&text);
        assert_eq!(m.get("kitaplik-0.1.0-linux-all.deb"), Some(&h));
        assert_eq!(m.get("Setup.exe"), Some(&"b".repeat(64)));
        assert_eq!(m.len(), 2);
    }
}
