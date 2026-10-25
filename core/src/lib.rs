//! C Launcher'ın işletim sisteminden bağımsız mantığı: katalog, sürüm karşılaştırma,
//! release dosyası seçimi, SHA-256 listesi ve uygulama durumu.

pub mod catalog;
pub mod paths;
pub mod select;
pub mod status;
pub mod sums;
pub mod version;

pub use catalog::{App, Catalog, Group, Method, PlatformSpec};
pub use status::Status;
pub use version::Version;

/// Launcher'ın çalıştığı işletim sistemi, katalogdaki `platforms` anahtarlarıyla aynı adlar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Macos,
    Linux,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Macos => "macos",
            Os::Linux => "linux",
        }
    }
}
