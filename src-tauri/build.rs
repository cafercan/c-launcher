fn main() {
    // Uygulamanın adı (tauri.conf.json productName) koda gömülür: kendini güncelleme,
    // yalnızca bu adla yayımlanmış paketleri kabul eder (bkz. selfupdate::is_ours).
    println!("cargo:rerun-if-changed=tauri.conf.json");
    let conf: serde_json::Value = serde_json::from_str(&std::fs::read_to_string("tauri.conf.json").expect("tauri.conf.json okunamadı")).expect("tauri.conf.json geçersiz");
    let name = conf["productName"].as_str().expect("tauri.conf.json'da productName yok");
    println!("cargo:rustc-env=PRODUCT_NAME={name}");
    tauri_build::build()
}
