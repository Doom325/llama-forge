//! Web UI embedding support for `rust-embed`.
//!
//! `rust-embed`'s derive needs the embed folder (`ui/build`) to EXIST at compile
//! time even in debug builds (where assets are served from the filesystem at
//! runtime). On a fresh checkout / CI it does not exist yet, so we create it —
//! an empty dir compiles fine and simply serves nothing until the UI is built.
//!
//! The UI is built automatically via `npm ci && npm run build` whenever node is
//! available. Set `LLAMAFORGE_SKIP_UI_BUILD=1` to skip (e.g. CI that builds the
//! UI in a separate step).

use std::path::Path;
use std::process::Command;

fn main() {
    // ui/src covers components, routes, app.html, and app.css (Tailwind entry).
    println!("cargo:rerun-if-changed=../../ui/src");
    println!("cargo:rerun-if-changed=../../ui/package.json");
    println!("cargo:rerun-if-changed=../../ui/svelte.config.js");
    println!("cargo:rerun-if-changed=../../ui/vite.config.ts");

    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let ui_dir = Path::new(&manifest_dir).join("../../ui");

    // Guarantee the embed folder exists so the rust-embed derive compiles even
    // when the UI has not been built (debug, CI, fresh checkout). Empty is fine.
    // .expect so a failure here is legible instead of resurfacing as a cryptic
    // E0599 "no `get`" from the derive.
    std::fs::create_dir_all(ui_dir.join("build")).expect("create ui/build for rust-embed");

    // Escape hatch for pipelines that build the UI in a separate step.
    if std::env::var("LLAMAFORGE_SKIP_UI_BUILD").is_ok() {
        return;
    }
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
