use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let ui_out = out.join("ui");
    fs::create_dir_all(&ui_out).unwrap();

    // Compile every data/ui/*.blp into OUT_DIR/ui/*.ui
    let mut blps = Vec::new();
    for entry in fs::read_dir("data/ui").unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "blp") {
            println!("cargo:rerun-if-changed={}", p.display());
            blps.push(p);
        }
    }
    let status = Command::new("blueprint-compiler")
        .arg("batch-compile")
        .arg(&ui_out)
        .arg("data/ui")
        .args(&blps)
        .status()
        .expect("blueprint-compiler not found (dnf install blueprint-compiler)");
    assert!(status.success(), "blueprint-compiler failed");

    // GSettings schema for uninstalled runs (main.rs points GSETTINGS_SCHEMA_DIR here in debug builds)
    let schema_dir = out.join("schemas");
    fs::create_dir_all(&schema_dir).unwrap();
    fs::copy(
        "data/io.github.djshiye.Clipperino.gschema.xml",
        schema_dir.join("io.github.djshiye.Clipperino.gschema.xml"),
    )
    .unwrap();
    println!("cargo:rerun-if-changed=data/io.github.djshiye.Clipperino.gschema.xml");
    let status = Command::new("glib-compile-schemas")
        .arg(&schema_dir)
        .status()
        .expect("glib-compile-schemas not found");
    assert!(status.success(), "glib-compile-schemas failed");

    println!("cargo:rerun-if-changed=data/clipperino.gresource.xml");
    println!("cargo:rerun-if-changed=data/style.css");
    glib_build_tools::compile_resources(
        &["data", out.to_str().unwrap()],
        "data/clipperino.gresource.xml",
        "clipperino.gresource",
    );
}
