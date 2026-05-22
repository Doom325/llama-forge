use llamaforge_core::config::{AppConfig, PriorityRulesConfig};
use llamaforge_core::types::ServerId;
use std::fs;
use std::path::PathBuf;

#[test]
fn test_parse_example_llamaforge_toml() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let example_path = manifest_dir.join("../../examples/llamaforge.toml");
    let content = fs::read_to_string(example_path).expect("failed to read example llamaforge.toml");
    let config: AppConfig =
        toml::from_str(&content).expect("failed to parse example llamaforge.toml");

    assert_eq!(config.global.http_port, 8000);
    assert!(config.servers.contains_key(&ServerId::from_config_id("llama-3-8b")));
}

#[test]
fn test_parse_example_priority_rules_toml() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let example_path = manifest_dir.join("../../examples/priority_rules.toml");
    let content =
        fs::read_to_string(example_path).expect("failed to read example priority_rules.toml");

    // priority_rules.toml only contains the PriorityRulesConfig part,
    // but AppConfig::priority_rules is a PriorityRulesConfig.
    // Usually these are loaded separately and merged.
    // For this test, we can just parse it into AppConfig if we wrap it or
    // parse it directly into PriorityRulesConfig if it's public.

    let config: PriorityRulesConfig =
        toml::from_str(&content).expect("failed to parse example priority_rules.toml");

    assert_eq!(config.task_types.get(&"chat".into()), Some(&200));
    assert_eq!(config.starvation.check_interval_secs, 5);
}
