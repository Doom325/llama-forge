use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process;

pub const LLAMAFORGE_EXAMPLE: &str = include_str!("../../../examples/llamaforge.toml");
pub const PRIORITY_RULES_EXAMPLE: &str = include_str!("../../../examples/priority_rules.toml");

pub fn run(out_path: &Path, rules_path: Option<&Path>, state_dir: &Path, force: bool) -> ! {
    if let Err(e) = fs::create_dir_all(state_dir) {
        eprintln!("error: failed to create state dir {}: {}", state_dir.display(), e);
        process::exit(78);
    }
    if let Ok(metadata) = fs::metadata(state_dir) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o750);
        if let Err(e) = fs::set_permissions(state_dir, perms) {
            eprintln!(
                "warning: could not set state dir to 0750: {}. You may want to chmod manually.",
                e
            );
        }
    }

    if out_path.exists() && !force {
        let mtime = fs::metadata(out_path).and_then(|m| m.modified()).ok();
        let date_str = match mtime {
            Some(t) => {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                dt.format("%Y-%m-%d %H:%M:%S").to_string()
            }
            None => "unknown".to_string(),
        };
        eprintln!(
            "error: existing config at {} last modified {}. Use --force to overwrite.",
            out_path.display(),
            date_str
        );
        process::exit(78);
    }

    if let Some(parent) = out_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            eprintln!("error: failed to create parent dir {}: {}", parent.display(), e);
            process::exit(78);
        }
    }

    if let Err(e) = fs::write(out_path, LLAMAFORGE_EXAMPLE) {
        eprintln!("error: failed to write config to {}: {}", out_path.display(), e);
        process::exit(78);
    }

    if let Some(r_path) = rules_path {
        if r_path.exists() && !force {
            eprintln!("error: existing rules at {}. Use --force to overwrite.", r_path.display());
            process::exit(78);
        }
        if let Some(parent) = r_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("error: failed to create parent dir {}: {}", parent.display(), e);
                process::exit(78);
            }
        }
        if let Err(e) = fs::write(r_path, PRIORITY_RULES_EXAMPLE) {
            eprintln!("error: failed to write rules to {}: {}", r_path.display(), e);
            process::exit(78);
        }
    }

    println!(
        "Wrote default config to {}. Edit `model_path`, `port`, `ctx_size` for your llama-server instances, then run `llamaforge`.",
        out_path.display()
    );
    process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_examples_match_repo() {
        let repo_config = fs::read_to_string("../../examples/llamaforge.toml").unwrap();
        let repo_rules = fs::read_to_string("../../examples/priority_rules.toml").unwrap();

        assert_eq!(repo_config, LLAMAFORGE_EXAMPLE);
        assert_eq!(repo_rules, PRIORITY_RULES_EXAMPLE);
    }
}
