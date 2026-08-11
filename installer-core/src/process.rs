//! Shared helper for shelling out to real system tools (parted, mkfs.*, limine, tar, ...).
//! Every privileged/mutating module goes through this so error formatting stays consistent.

use tokio::process::Command;

pub async fn run(cmd: &str, args: &[&str]) -> crate::Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .await
        .map_err(|e| crate::Error::Command {
            cmd: format!("{cmd} {}", args.join(" ")),
            detail: e.to_string(),
        })?;

    if !output.status.success() {
        return Err(crate::Error::Command {
            cmd: format!("{cmd} {}", args.join(" ")),
            detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Same as `run`, but for commands whose stdout we don't care about (mkfs, mount, ...).
pub async fn run_status(cmd: &str, args: &[&str]) -> crate::Result<()> {
    run(cmd, args).await.map(|_| ())
}
