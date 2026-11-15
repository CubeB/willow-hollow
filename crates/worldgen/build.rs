//! Gathers the structures saved from the structure editor, under `assets/structures` at
//! the top of the repository, so they are compiled in: the world then depends only on
//! the source it was built from, never on files found beside the game at run time.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../../assets/structures");
    // Cargo looks through a folder named here for any change. The folder is kept in the
    // repository (it has a README), as a missing one would rerun this every build.
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    if let Ok(kinds) = fs::read_dir(&root) {
        for kind in kinds.flatten() {
            if !kind.path().is_dir() {
                continue;
            }
            for file in fs::read_dir(kind.path()).into_iter().flatten().flatten() {
                let path = file.path();
                if path.extension().is_some_and(|e| e == "txt") {
                    files.push(path);
                }
            }
        }
    }
    files.sort();
    let mut out = String::from("pub static FILES: &[(&str, &str)] = &[\n");
    for path in &files {
        let rel = relative(&root, path);
        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        out += &format!("    ({rel:?}, include_str!({:?})),\n", abs.to_string_lossy());
    }
    out += "];\n";
    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("saved_structures.rs");
    fs::write(dest, out).unwrap();
}

/// `path` under `root`, with forward slashes: `shrine/high-gate.txt`.
fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}
