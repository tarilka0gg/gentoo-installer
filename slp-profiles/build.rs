// Compiles the files of ../profiles into the crate (`BUNDLE`: path relative to profiles/, contents; `BUNDLE_IMAGES`: the screenshots as
// bytes), so that neither the installer nor `slp` needs the data files at run time.
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .expect("profiles directory")
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else {
            let rel = p
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel != "README.md" {
                out.push((rel, p));
            }
        }
    }
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let base = manifest
        .join("../profiles")
        .canonicalize()
        .expect("../profiles");
    println!("cargo:rerun-if-changed={}", base.display());
    let mut files = Vec::new();
    walk(&base, &base, &mut files);
    let is_image =
        |rel: &str| rel.ends_with(".jpg") || rel.ends_with(".webp") || rel.ends_with(".png");
    let mut code = String::from("pub static BUNDLE: &[(&str, &str)] = &[\n");
    for (rel, path) in files.iter().filter(|(r, _)| !is_image(r)) {
        code.push_str(&format!(
            "    ({:?}, include_str!({:?})),\n",
            rel,
            path.display().to_string()
        ));
    }
    code.push_str("];\n");
    // the screenshots: bytes
    code.push_str("pub static BUNDLE_IMAGES: &[(&str, &[u8])] = &[\n");
    for (rel, path) in files.iter().filter(|(r, _)| is_image(r)) {
        code.push_str(&format!(
            "    ({:?}, include_bytes!({:?})),\n",
            rel,
            path.display().to_string()
        ));
    }
    code.push_str("];\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("bundle.rs"),
        code,
    )
    .unwrap();
}
