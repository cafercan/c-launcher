//! Linux: `deb` yöntemindeki bir uygulamanın bu dağıtımda nasıl kurulacağı.
//!
//! `.deb` yalnızca apt/dpkg olan sistemlerde (Debian, Ubuntu, Mint…) kurulabilir. Arch gibi
//! başka dağıtımlarda uygulama release'inde bir pacman paketi (`.pkg.tar.zst`) sunuyorsa o,
//! yoksa taşınabilir `.tar.gz` paketi kullanılır. İkisi de yoksa uygulama o sistemde sunulmuyor
//! demektir.

use crate::PlatformSpec;

/// Sistemde bulunan paket yöneticileri.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PackageManagers {
    /// `apt-get` ve `dpkg-query`
    pub apt: bool,
    /// `pacman`
    pub pacman: bool,
}

/// Bir `deb` uygulamasının kurulum yolu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// `.deb`, apt-get ile
    Apt,
    /// `.pkg.tar.zst`, pacman ile
    Pacman,
    /// `.tar.gz`, bir klasöre açılır; parola gerekmez
    Portable,
}

/// Uygulamanın şu anda nasıl kurulu olduğu (kurulu değilse `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstalledVia {
    Dpkg,
    Pacman,
    Portable,
}

/// Kurulum ya da güncelleme için yol seçer. Güncellemeler uygulama nasıl kurulduysa öyle
/// yapılır; yeni kurulumlarda kullanıcı bir kurulum klasörü seçtiyse taşınabilir paket
/// öncelikli, değilse sistemin paket yöneticisi.
pub fn plan(spec: &PlatformSpec, before: Option<InstalledVia>, root_set: bool, pm: PackageManagers) -> Option<Plan> {
    let portable = spec.portable.is_some();
    let pacman = pm.pacman && spec.pacman.is_some();
    match before {
        Some(InstalledVia::Portable) => portable.then_some(Plan::Portable),
        Some(InstalledVia::Pacman) => pacman.then_some(Plan::Pacman),
        Some(InstalledVia::Dpkg) => pm.apt.then_some(Plan::Apt),
        None if root_set && portable => Some(Plan::Portable),
        None if pm.apt => Some(Plan::Apt),
        None if pacman => Some(Plan::Pacman),
        None if portable => Some(Plan::Portable),
        None => None,
    }
}

/// Seçilen yol için release dosyası kalıbı.
pub fn asset_pattern(spec: &PlatformSpec, plan: Plan) -> Option<&str> {
    match plan {
        Plan::Apt => spec.asset.as_deref(),
        Plan::Pacman => spec.pacman.as_ref().map(|p| p.asset.as_str()),
        Plan::Portable => spec.portable.as_ref().map(|p| p.asset.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Pacman, Portable};
    use crate::Method;

    fn spec(portable: bool, pacman: bool) -> PlatformSpec {
        let mut s: PlatformSpec = serde_json::from_str(r#"{"method":"deb","asset":"a_*.deb","package":"a"}"#).unwrap();
        s.method = Method::Deb;
        if portable {
            s.portable = Some(Portable { asset: "a-*.tar.gz".into(), run: "a/a".into(), icon: None, wm_class: None });
        }
        if pacman {
            s.pacman = Some(Pacman { asset: "a-*.pkg.tar.zst".into(), package: "a-bin".into() });
        }
        s
    }

    const DEBIAN: PackageManagers = PackageManagers { apt: true, pacman: false };
    const ARCH: PackageManagers = PackageManagers { apt: false, pacman: true };
    const OTHER: PackageManagers = PackageManagers { apt: false, pacman: false };

    #[test]
    fn debian_keeps_old_behaviour() {
        assert_eq!(plan(&spec(true, true), None, false, DEBIAN), Some(Plan::Apt));
        assert_eq!(plan(&spec(true, true), None, true, DEBIAN), Some(Plan::Portable));
        assert_eq!(plan(&spec(false, false), None, true, DEBIAN), Some(Plan::Apt));
    }

    #[test]
    fn arch_prefers_pacman_then_portable() {
        assert_eq!(plan(&spec(true, true), None, false, ARCH), Some(Plan::Pacman));
        assert_eq!(plan(&spec(true, false), None, false, ARCH), Some(Plan::Portable));
        assert_eq!(plan(&spec(true, true), None, true, ARCH), Some(Plan::Portable));
        // yalnızca .deb sunan uygulama Arch'ta kurulamaz
        assert_eq!(plan(&spec(false, false), None, false, ARCH), None);
    }

    #[test]
    fn other_distros_use_portable_only() {
        assert_eq!(plan(&spec(true, true), None, false, OTHER), Some(Plan::Portable));
        assert_eq!(plan(&spec(false, true), None, false, OTHER), None);
    }

    #[test]
    fn updates_follow_the_existing_install() {
        assert_eq!(plan(&spec(true, true), Some(InstalledVia::Portable), false, DEBIAN), Some(Plan::Portable));
        assert_eq!(plan(&spec(true, true), Some(InstalledVia::Pacman), true, ARCH), Some(Plan::Pacman));
        assert_eq!(plan(&spec(true, true), Some(InstalledVia::Dpkg), true, DEBIAN), Some(Plan::Apt));
        // release artık o biçimi sunmuyorsa başka biçime geçilmez
        assert_eq!(plan(&spec(true, false), Some(InstalledVia::Pacman), false, ARCH), None);
    }

    #[test]
    fn patterns_follow_the_plan() {
        let s = spec(true, true);
        assert_eq!(asset_pattern(&s, Plan::Apt), Some("a_*.deb"));
        assert_eq!(asset_pattern(&s, Plan::Pacman), Some("a-*.pkg.tar.zst"));
        assert_eq!(asset_pattern(&s, Plan::Portable), Some("a-*.tar.gz"));
    }
}
