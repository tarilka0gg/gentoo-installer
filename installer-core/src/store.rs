//! Wires the installed system to the distro's own Portage overlay + binhost
//! (the "portage store" this installer exists to sit in front of).

use crate::command::CommandRunner;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub binhost_url: String,
    pub overlay_git_url: String,
    pub overlay_name: String,
}

/// Writes `/etc/portage/repos.conf/<overlay_name>.conf` and
/// `/etc/portage/binrepos.conf/<overlay_name>.conf` into the target root, then clones the
/// overlay directly with `git` — not `emerge --sync`. Per spec, the installer never
/// invokes emerge; `auto-sync = yes` in the written repos.conf means Portage picks the
/// overlay up and keeps it current on the *installed* system's own first sync, the
/// installer just needs it present so the app store / first `emerge` on first boot has
/// something to work with immediately.
pub async fn configure(
    runner: &dyn CommandRunner,
    target_root: &Path,
    cfg: &StoreConfig,
) -> crate::Result<()> {
    let repos_conf_dir = target_root.join("etc/portage/repos.conf");
    let binrepos_conf_dir = target_root.join("etc/portage/binrepos.conf");
    tokio::fs::create_dir_all(&repos_conf_dir).await?;
    tokio::fs::create_dir_all(&binrepos_conf_dir).await?;

    let repos_conf = format!(
        "[{name}]\n\
         location = /var/db/repos/{name}\n\
         sync-type = git\n\
         sync-uri = {url}\n\
         auto-sync = yes\n",
        name = cfg.overlay_name,
        url = cfg.overlay_git_url,
    );
    tokio::fs::write(
        repos_conf_dir.join(format!("{}.conf", cfg.overlay_name)),
        repos_conf,
    )
    .await?;

    let binrepos_conf = format!(
        "[{name}]\n\
         sync-uri = {url}\n",
        name = cfg.overlay_name,
        url = cfg.binhost_url,
    );
    tokio::fs::write(
        binrepos_conf_dir.join(format!("{}.conf", cfg.overlay_name)),
        binrepos_conf,
    )
    .await?;

    let repo_dir = target_root.join(format!("var/db/repos/{}", cfg.overlay_name));
    tokio::fs::create_dir_all(repo_dir.parent().expect("var/db/repos always has a parent")).await?;
    let repo_dir_str = repo_dir
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 overlay path")))?;
    runner
        .run_status(
            "git",
            &["clone", "--depth", "1", &cfg.overlay_git_url, repo_dir_str],
        )
        .await?;

    Ok(())
}

/// Lists atoms currently published on the binhost — used by `kernel::resolve` to check
/// which hardware-profile kernel packages actually exist before picking one.
/// Parses the Portage binary-package-index format (`PMS` binpkg-multi-instance `Packages`
/// file): entries are blank-line-separated stanzas, each with a `CPV: cat/pkg-version` line.
pub async fn list_binhost_atoms(binhost_url: &str) -> crate::Result<Vec<String>> {
    let url = format!("{}/Packages", binhost_url.trim_end_matches('/'));
    let body = reqwest::get(&url)
        .await
        .map_err(|e| crate::Error::Other(e.into()))?
        .error_for_status()
        .map_err(|e| crate::Error::Other(e.into()))?
        .text()
        .await
        .map_err(|e| crate::Error::Other(e.into()))?;

    Ok(body
        .lines()
        .filter_map(|l| l.strip_prefix("CPV: "))
        .map(|cpv| {
            // CPV includes the version (cat/pkg-1.2.3); strip it back to the bare atom
            // category/name, which is what kernel::resolve matches against.
            strip_version_suffix(cpv)
        })
        .collect())
}

fn strip_version_suffix(cpv: &str) -> String {
    match cpv.rsplit_once('-') {
        Some((base, ver)) if ver.starts_with(|c: char| c.is_ascii_digit()) => base.to_string(),
        _ => cpv.to_string(),
    }
}
