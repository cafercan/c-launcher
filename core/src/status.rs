//! Kartta görünen durum: kurulu mu, güncelleme var mı, bu sistem için sunuluyor mu.

use serde::Serialize;

use crate::Version;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Bu işletim sistemi için tarif ya da release dosyası yok
    Unsupported,
    /// Henüz hiç release yayımlanmamış
    NoRelease,
    NotInstalled,
    UpToDate,
    UpdateAvailable,
}

pub fn compute(supported: bool, latest: Option<&Version>, installed: Option<&Version>, installed_unknown: bool) -> Status {
    if !supported {
        return Status::Unsupported;
    }
    match (installed, latest) {
        (Some(i), Some(l)) if l > i => Status::UpdateAvailable,
        (Some(_), _) => Status::UpToDate,
        // kurulu ama sürümü okunamadı: güncellemeyi zorlamayalım
        (None, _) if installed_unknown => Status::UpToDate,
        (None, None) => Status::NoRelease,
        (None, Some(_)) => Status::NotInstalled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn computes_status() {
        assert_eq!(compute(false, Some(&v("1.0")), None, false), Status::Unsupported);
        assert_eq!(compute(true, None, None, false), Status::NoRelease);
        assert_eq!(compute(true, Some(&v("1.0")), None, false), Status::NotInstalled);
        assert_eq!(compute(true, Some(&v("1.1")), Some(&v("1.0")), false), Status::UpdateAvailable);
        assert_eq!(compute(true, Some(&v("1.0")), Some(&v("1.0")), false), Status::UpToDate);
        assert_eq!(compute(true, Some(&v("1.0")), Some(&v("1.2")), false), Status::UpToDate);
        assert_eq!(compute(true, Some(&v("1.0")), None, true), Status::UpToDate);
    }
}
