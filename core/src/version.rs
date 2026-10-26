//! Sürüm numaraları: "v1.2.0", "1.2", "2.9.0-beta.1" gibi etiketleri karşılaştırır.

use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone)]
pub struct Version {
    parts: Vec<u64>,
    /// "-beta.1" gibi ön sürüm eki; ön sürüm, aynı numaralı kararlı sürümden küçüktür
    pre: Option<String>,
    raw: String,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let raw = s.trim();
        let body = raw.strip_prefix(['v', 'V']).unwrap_or(raw);
        let (nums, pre) = match body.split_once(['-', '+']) {
            Some((n, rest)) if body.as_bytes()[n.len()] == b'-' => (n, Some(rest.split('+').next().unwrap_or("").to_string())),
            Some((n, _)) => (n, None),
            None => (body, None),
        };
        let parts = nums.split('.').map(|p| p.parse::<u64>().ok()).collect::<Option<Vec<_>>>()?;
        if parts.is_empty() {
            return None;
        }
        Some(Version { parts, pre: pre.filter(|p| !p.is_empty()), raw: body.to_string() })
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Ön sürüm eki ("-beta.1") yok
    pub fn is_stable(&self) -> bool {
        self.pre.is_none()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let n = self.parts.len().max(other.parts.len());
        for i in 0..n {
            let a = self.parts.get(i).copied().unwrap_or(0);
            let b = other.parts.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                Ordering::Equal => continue,
                o => return o,
            }
        }
        match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => a.cmp(b),
        }
    }
}

// "1.2" ile "1.2.0" ve "1.2.0+build5" eşittir: eşitlik sıralamayla aynı kurala uyar
impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn parses_common_tags() {
        assert_eq!(v("v2.9.0").as_str(), "2.9.0");
        assert_eq!(v("0.1.4").as_str(), "0.1.4");
        assert!(Version::parse("latest").is_none());
        assert!(Version::parse("").is_none());
    }

    #[test]
    fn orders_versions() {
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.2") == v("1.2.0"));
        assert!(v("v1.2.0") > v("1.2.0-beta.1"));
        assert!(v("1.2.0-beta.2") > v("1.2.0-beta.1"));
        assert!(v("1.2.0+build5") == v("1.2.0"));
    }
}
