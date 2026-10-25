//! Launcher'ın kendi veri klasörü: indirmeler, açılmış eklentiler, terminal araçları,
//! önbellek ve kurulu sürümlerin kaydı (`state.json`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Record {
    /// Launcher'ın kurduğu sürüm (betik ve Chrome yöntemleri için kurulu sürümün kaynağı)
    pub version: String,
    /// Chrome eklentisi güncellendi ama kullanıcı henüz Chrome'da yenilemedi
    #[serde(default)]
    pub needs_reload: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StateFile {
    #[serde(default)]
    apps: BTreeMap<String, Record>,
}

pub struct Store {
    root: PathBuf,
    state: Mutex<StateFile>,
}

impl Store {
    pub fn open() -> Store {
        let root = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("c-launcher");
        let _ = std::fs::create_dir_all(&root);
        let state = std::fs::read_to_string(root.join("state.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Store { root, state: Mutex::new(state) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dir(&self, name: &str) -> PathBuf {
        let d = self.root.join(name);
        let _ = std::fs::create_dir_all(&d);
        d
    }

    pub fn record(&self, id: &str) -> Option<Record> {
        self.state.lock().unwrap().apps.get(id).cloned()
    }

    pub fn set_record(&self, id: &str, record: Option<Record>) {
        let mut st = self.state.lock().unwrap();
        match record {
            Some(r) => st.apps.insert(id.to_string(), r),
            None => st.apps.remove(id),
        };
        // önce geçici dosyaya yaz: yarıda kalan yazma kaydı bozmasın
        if let Ok(json) = serde_json::to_string_pretty(&*st) {
            let tmp = self.root.join("state.json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, self.root.join("state.json"));
            }
        }
    }
}
