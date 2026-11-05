//! Release'teki `SHA256SUMS.txt` dosyası (`sha256sum` çıktısı biçiminde). Bazı release'ler
//! özetleri sisteme göre ayrı dosyalara böler (`SHA256SUMS-linux.txt`, `SHA256SUMS-arch.txt`);
//! onlar da okunur.

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

/// Release dosyası bir özet listesi mi: `SHA256SUMS.txt` ya da `SHA256SUMS-<ek>.txt`.
pub fn is_sums_file(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "sha256sums.txt" || (n.starts_with("sha256sums-") && n.ends_with(".txt") && n.len() > "sha256sums-.txt".len())
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

    #[test]
    fn recognises_split_sums_files() {
        assert!(is_sums_file("SHA256SUMS.txt"));
        assert!(is_sums_file("sha256sums.txt"));
        assert!(is_sums_file("SHA256SUMS-arch.txt"));
        assert!(is_sums_file("SHA256SUMS-linux.txt"));
        assert!(!is_sums_file("SHA256SUMS-.txt"));
        assert!(!is_sums_file("SHA256SUMS.txt.sig"));
        assert!(!is_sums_file("notes.txt"));
    }
}
