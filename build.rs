//! Lists the documentation and example files so they are compiled into the binary (the "Documentation" window
//! renders them without needing the source tree). The list is the directory's contents, so a new page is picked up
//! by itself.

use std::fmt::Write;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, ext: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, ext, out);
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| ext.contains(&x)) {
            out.push(p);
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let mut files = vec![root.join("README.md")];
    collect(&root.join("docs"), &["md"], &mut files);
    collect(&root.join("examples"), &["yaml"], &mut files);
    files.sort();
    let mut src = String::from("pub const FILES: &[(&str, &str)] = &[\n");
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        writeln!(src, "    ({rel:?}, include_str!({:?})),", f.display().to_string()).unwrap();
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("docs_files.rs");
    std::fs::write(out, src).unwrap();
    println!("cargo:rerun-if-changed=docs");
    println!("cargo:rerun-if-changed=examples");
    println!("cargo:rerun-if-changed=README.md");
    println!("cargo:rerun-if-changed=build.rs");
}
