use std::path::Path;
use std::process::Command;

pub fn origin_url(cwd: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8(output.stdout).ok()?;
    let url = url.trim();
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}
