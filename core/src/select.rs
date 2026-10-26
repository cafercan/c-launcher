//! Release dosyaları arasından katalogdaki kalıba uyanı seçer.

/// `*` herhangi bir karakter dizisiyle eşleşir; büyük/küçük harf duyarsızdır.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ni < n.len() {
        if pi < p.len() && p[pi] != '*' && p[pi] == n[ni] {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Kalıba uyan ilk dosyanın adını döndürür. Birden çok eşleşme varsa sonuç
/// belirsiz kalmasın diye ada göre sıralanmış listedeki ilki seçilir.
pub fn pick<'a>(pattern: &str, names: &[&'a str]) -> Option<&'a str> {
    let mut hits: Vec<&str> = names.iter().copied().filter(|n| wildcard(pattern, n)).collect();
    hits.sort_unstable();
    hits.first().copied()
}

/// Etiketin sürüm kısmı: ("alpha-v0.1.5", Some("alpha-")) -> "v0.1.5".
/// Önek yoksa ya da etiket önekle başlamıyorsa etiket olduğu gibi döner.
pub fn strip_tag<'a>(tag: &'a str, prefix: Option<&str>) -> &'a str {
    prefix.and_then(|p| tag.strip_prefix(p)).unwrap_or(tag)
}

/// Aynı depoyu paylaşan uygulamalardan birinin en yeni kararlı release'i: etiketi
/// `prefix` + sürüm numarası olan, taslak ya da ön sürüm olmayanların en büyüğü.
/// `tags`: (etiket, taslak ya da ön sürüm mü)
pub fn pick_release<'a>(tags: &[(&'a str, bool)], prefix: &str) -> Option<&'a str> {
    tags.iter()
        .filter(|(_, unstable)| !unstable)
        .filter_map(|(tag, _)| {
            let v = crate::Version::parse(tag.strip_prefix(prefix)?)?;
            // "-beta.1" gibi ekli etiketler de kararlı sayılmaz
            v.is_stable().then_some((v, *tag))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, tag)| tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_release_by_prefix() {
        let tags = [
            ("alpha-v0.1.5", false),
            ("alpha-v0.1.10", false),
            ("beta-v20260915", false),
            ("alpha-v0.2.0-beta.1", true),
            ("alpha-v0.3.0-rc.1", false),
            ("alpha-notes", false),
        ];
        assert_eq!(pick_release(&tags, "alpha-"), Some("alpha-v0.1.10"));
        assert_eq!(pick_release(&tags, "beta-"), Some("beta-v20260915"));
        assert_eq!(pick_release(&tags, "al-"), None);
    }

    #[test]
    fn strip_tag_cases() {
        assert_eq!(strip_tag("alpha-v0.1.5", Some("alpha-")), "v0.1.5");
        assert_eq!(strip_tag("v0.1.5", None), "v0.1.5");
        assert_eq!(strip_tag("other-v1", Some("alpha-")), "other-v1");
    }

    #[test]
    fn matches_wildcards() {
        assert!(wildcard("kitaplik-*-windows-x64-setup.exe", "kitaplik-0.1.0-windows-x64-setup.exe"));
        assert!(wildcard("MDViewer-*.exe", "mdviewer-v2.9.0.exe"));
        assert!(!wildcard("MDViewer-*.exe", "MDViewer-v2.9.0.exe.sig"));
        assert!(wildcard("*", "anything"));
        assert!(wildcard("a*b*c", "aXXbYYc"));
        assert!(!wildcard("a*b*c", "aXXbYY"));
    }

    #[test]
    fn picks_matching_asset() {
        let assets = ["mdflowviewer_2.9.0_all.deb", "MDViewer-v2.9.0.exe", "SHA256SUMS.txt"];
        assert_eq!(pick("mdflowviewer_*_all.deb", &assets), Some("mdflowviewer_2.9.0_all.deb"));
        assert_eq!(pick("*.dmg", &assets), None);
    }
}
