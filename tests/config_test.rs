//! Integration tests for `config::load`.

mod common;

use common::unique_path;
use gitwell::config;
use std::fs;

#[test]
fn parses_all_known_fields_from_gitwell_toml() {
    let dir = unique_path("config-all");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join(".gitwell.toml"),
        r#"
# full config sample
stale_days = 14
dormant_months = 3
session_window_hours = 72
ignore_repos = ["node_modules", ".cache"]
ignore_branches = ["dependabot/*", "renovate/*"]
"#,
    )
    .unwrap();

    let c = config::load(&dir);

    assert_eq!(c.stale_days, 14);
    assert_eq!(c.dormant_months, 3);
    assert_eq!(c.session_window_hours, 72);
    assert_eq!(c.ignore_repos, vec!["node_modules", ".cache"]);
    assert_eq!(c.ignore_branches, vec!["dependabot/*", "renovate/*"]);
    assert_eq!(
        c.loaded_from.as_deref(),
        Some(dir.join(".gitwell.toml").as_path())
    );

    // Cleanup
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_config_yields_defaults() {
    let dir = unique_path("config-missing");
    fs::create_dir_all(&dir).unwrap();
    // No config file written.

    let c = config::load(&dir);

    assert_eq!(c.stale_days, 30);
    assert_eq!(c.dormant_months, 6);
    assert_eq!(c.session_window_hours, 48);
    assert!(c.ignore_repos.is_empty());
    assert!(c.ignore_branches.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn inline_comments_are_stripped() {
    let dir = unique_path("config-comments");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(".gitwell.toml"),
        "stale_days = 21 # override default\ndormant_months = 2 # aggressive\n",
    )
    .unwrap();

    let c = config::load(&dir);
    assert_eq!(c.stale_days, 21);
    assert_eq!(c.dormant_months, 2);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn string_array_parsing_handles_quoted_values_with_spaces() {
    let dir = unique_path("config-arrays");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(".gitwell.toml"),
        r#"ignore_repos = [ "repo one" , "repo-two" , "repo_three" ]"#,
    )
    .unwrap();

    let c = config::load(&dir);
    assert_eq!(
        c.ignore_repos,
        vec![
            "repo one".to_string(),
            "repo-two".to_string(),
            "repo_three".to_string()
        ]
    );

    let _ = fs::remove_dir_all(&dir);
}
