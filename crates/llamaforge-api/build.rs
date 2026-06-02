//! Builds and embeds the web UI for RELEASE builds only.
//!
//! Debug builds skip this: `rust-embed` serves `ui/build` from the filesystem
//! at runtime, so `cargo build` / CI need neither node nor a pre-built UI.
//! Release builds run the production UI build so `rust-embed` can embed it.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../ui/src");
    println!("cargo:rerun-if-changed=../../ui/package.json");
    println!("cargo:rerun-if-changed=../../ui/svelte.config.js");

    if std::env::var("PROFILE").as_deref() != Ok("release") {
        return;
    }
    // Escape hatch for pipelines that build the UI in a separate step.
    if std::env::var("LLAMAFORGE_SKIP_UI_BUILD").is_ok() {
        return;
    }

    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let ui_dir = Path::new(&manifest_dir).join("../../ui");
    if !ui_dir.join("package.json").exists() {
        println!("cargo:warning=ui/ not found; the web UI will not be embedded");
        return;
    }

    run(&ui_dir, "npm", &["ci"]);
    run(&ui_dir, "npm", &["run", "build"]);
}

fn run(dir: &Path, cmd: &str, args: &[&str]) {
    match Command::new(cmd).current_dir(dir).args(args).status() {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("`{cmd} {}` exited with {status}", args.join(" ")),
        Err(err) => panic!("failed to spawn `{cmd} {}`: {err}", args.join(" ")),
    }
}
