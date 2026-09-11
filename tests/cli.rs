use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fji"))
}

#[test]
fn help_is_human_stdout() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Agent-first Forgejo issue CLI"));
    assert!(out.stderr.is_empty());
}

#[test]
fn version_is_human_stdout() {
    let out = bin().arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("fji"));
}

#[test]
fn missing_title_is_json_usage() {
    let out = bin().args(["create", "--body", "x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let err: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["code"], "usage");
}

#[test]
fn all_and_limit_conflict() {
    let out = bin()
        .args(["list", "--all", "--limit", "10"])
        .env_remove("FJI_TOKEN")
        .env_remove("FORGEJO_TOKEN")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["code"], "usage");
}

#[test]
fn limit_zero_rejected() {
    let out = bin().args(["list", "--limit", "0"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["code"], "usage");
}

#[test]
fn missing_token_json() {
    let out = bin()
        .args(["-H", "git.example", "-R", "sid/hello", "view", "1"])
        .env_remove("FJI_TOKEN")
        .env_remove("FORGEJO_TOKEN")
        .current_dir("/tmp")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["code"], "no_token");
}
