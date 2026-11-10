//! Embed `web/` into the binary: generates a static table of (path, bytes).

use std::path::{Path, PathBuf};

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            println!("cargo:rerun-if-changed={}", p.display());
            walk(&p, base, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            let key = rel.to_string_lossy().replace('\\', "/");
            println!("cargo:rerun-if-changed={}", p.display());
            out.push((key, p));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let web = manifest
        .join("../../web")
        .canonicalize()
        .expect("web/ directory next to crates/");
    println!("cargo:rerun-if-changed={}", web.display());
    let mut files = Vec::new();
    walk(&web, &web, &mut files);
    let mut code = String::from("pub static ASSETS: &[(&str, &[u8])] = &[\n");
    for (key, path) in files {
        code.push_str(&format!(
            "    ({key:?}, include_bytes!({:?})),\n",
            path.display().to_string()
        ));
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("assets.rs");
    std::fs::write(out, code).unwrap();
}
