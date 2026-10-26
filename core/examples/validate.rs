//! Bir catalog.json dosyasını launcher'ın kurallarıyla denetler. Katalog depolarının CI'ı için:
//! `cargo run -p launcher-core --example validate -- catalog.json`

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("kullanım: validate <catalog.json>");
        std::process::exit(2);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{path} okunamadı: {e}");
            std::process::exit(1);
        }
    };
    match launcher_core::Catalog::parse(&text) {
        Ok(_) => println!("ok"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
