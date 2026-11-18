//! Gathers the structures saved from the structure editor, under `assets/structures` at
//! the top of the repository, so they are compiled in: the world then depends only on
//! the source it was built from, never on files found beside the game at run time.
//!
//! The house kit's rules (`assets/structures/kit/<region>.txt`) are not structures, and go
//! into a list of their own for `settlements::kit`.

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
    let mut kit = Vec::new();
    if let Ok(kinds) = fs::read_dir(&root) {
        for kind in kinds.flatten() {
            if !kind.path().is_dir() {
                continue;
            }
            let is_kit = kind.file_name() == "kit";
            for file in fs::read_dir(kind.path()).into_iter().flatten().flatten() {
                let path = file.path();
                if path.extension().is_some_and(|e| e == "txt") {
                    if is_kit { kit.push(path) } else { files.push(path) }
                }
            }
        }
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out_dir.join("saved_structures.rs"), listing(&root, "FILES", files)).unwrap();
    fs::write(out_dir.join("kit_rules.rs"), listing(&root, "KIT", kit)).unwrap();
}

/// A static named `name` listing each file's path under `root` and its text, in order.
fn listing(root: &Path, name: &str, mut files: Vec<PathBuf>) -> String {
    files.sort();
    let mut out = format!("pub static {name}: &[(&str, &str)] = &[\n");
    for path in &files {
        let rel = relative(root, path);
        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        out += &format!("    ({rel:?}, include_str!({:?})),\n", abs.to_string_lossy());
    }
    out += "];\n";
    out
}

/// `path` under `root`, with forward slashes: `shrine/high-gate.txt`.
fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}
