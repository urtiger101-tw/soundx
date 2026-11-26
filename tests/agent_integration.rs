use serde_json::Value;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn uninstall_reports_conflicts_and_continues_with_other_agents() {
    let home = std::env::temp_dir().join(format!(
        "soundx-agent-cli-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&home).unwrap();
    for agent in ["codex", "claude", "opencode"] {
        let output = Command::new(env!("CARGO_BIN_EXE_soundx"))
            .args(["integrate", "install", "--agent", agent, "--home"])
            .arg(&home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let invalid_config = home.join(".codex/config.toml");
    std::fs::write(&invalid_config, "[invalid TOML").unwrap();
    let custom_skill = home.join(".claude/skills/soundx/SKILL.md");
    std::fs::write(&custom_skill, "User customization").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_soundx"))
        .args(["integrate", "remove", "--all", "--home"])
        .arg(&home)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let results: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    let status = |name| {
        results.iter().find(|value| value["agent"] == name).unwrap()["status"]
            .as_str()
            .unwrap()
    };
    assert_eq!(status("codex"), "failed");
    assert_eq!(status("claude"), "modified_files_preserved");
    assert_eq!(status("opencode"), "removed");
    assert_eq!(status("agy"), "not_installed");
    assert_eq!(
        std::fs::read_to_string(invalid_config).unwrap(),
        "[invalid TOML"
    );
    assert_eq!(
        std::fs::read_to_string(custom_skill).unwrap(),
        "User customization"
    );
    assert!(
        !home
            .join(".config/opencode/skills/soundx/SKILL.md")
            .exists()
    );
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn list_and_remove_do_not_create_a_missing_home_but_install_may() {
    let base = std::env::temp_dir().join(format!(
        "soundx-agent-home-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let missing = base.join("typo/home");
    for args in [
        vec!["integrate", "list", "--home"],
        vec!["integrate", "remove", "--all", "--home"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_soundx"))
            .args(&args)
            .arg(&missing)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
        assert!(!base.exists(), "{args:?} created the home");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_soundx"))
        .args(["integrate", "install", "--agent", "codex", "--home"])
        .arg(&missing)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(missing.join(".codex/config.toml").is_file());
    std::fs::remove_dir_all(base).unwrap();
}
