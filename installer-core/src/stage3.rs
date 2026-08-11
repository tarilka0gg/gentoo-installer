//! Fetches and unpacks an official Gentoo stage3 tarball (amd64, openrc, multilib, non-hardened).

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncWriteExt;

pub const DEFAULT_MIRROR: &str = "https://distfiles.gentoo.org/releases/amd64/autobuilds";
const INDEX_FILE: &str = "latest-stage3-amd64-openrc.txt";

#[derive(Debug, Clone)]
pub struct Stage3Source {
    pub url: String,
    pub sha256: Option<String>,
}

/// Reads the mirror's `latest-stage3-amd64-openrc.txt` index to resolve the current
/// tarball filename (Gentoo autobuilds are published under a date-stamped path), then
/// fetches the companion `.DIGESTS` file to pull a SHA256 if the index publishes one.
pub async fn resolve_latest() -> crate::Result<Stage3Source> {
    resolve_latest_from(DEFAULT_MIRROR).await
}

pub async fn resolve_latest_from(mirror: &str) -> crate::Result<Stage3Source> {
    let index_url = format!("{mirror}/{INDEX_FILE}");
    let body = reqwest::get(&index_url)
        .await
        .map_err(|e| crate::Error::Other(e.into()))?
        .error_for_status()
        .map_err(|e| crate::Error::Other(e.into()))?
        .text()
        .await
        .map_err(|e| crate::Error::Other(e.into()))?;

    let relative_path = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
        .next_back()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("empty or unparsable {INDEX_FILE}")))?;

    let url = format!("{mirror}/{relative_path}");
    let sha256 = fetch_sha256_digest(&format!("{url}.DIGESTS")).await;

    Ok(Stage3Source { url, sha256 })
}

/// `.DIGESTS` files list several hash algorithms; we only care about the SHA256 block,
/// formatted as `# SHA256 HASH\n<hex>  <filename>`. Missing/unparsable digest is not fatal —
/// the caller can still proceed unverified, since https + the official mirror is already
/// a meaningful trust boundary.
async fn fetch_sha256_digest(url: &str) -> Option<String> {
    let body = reqwest::get(url).await.ok()?.text().await.ok()?;
    let mut lines = body.lines();
    while let Some(line) = lines.next() {
        if line.trim_start_matches('#').trim().eq_ignore_ascii_case("SHA256") {
            let hash_line = lines.next()?;
            return hash_line.split_whitespace().next().map(str::to_string);
        }
    }
    None
}

pub async fn download(source: &Stage3Source, dest: &Path) -> crate::Result<()> {
    let response = reqwest::get(&source.url)
        .await
        .map_err(|e| crate::Error::Other(e.into()))?
        .error_for_status()
        .map_err(|e| crate::Error::Other(e.into()))?;

    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = Sha256::new();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| crate::Error::Other(e.into()))?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;

    if let Some(expected) = &source.sha256 {
        let actual = hex::encode(hasher.finalize());
        if &actual != expected {
            return Err(crate::Error::Other(anyhow::anyhow!(
                "stage3 sha256 mismatch: expected {expected}, got {actual}"
            )));
        }
    }

    Ok(())
}

/// Unpacks the tarball into `root` (typically the mounted target `@` subvolume),
/// preserving ownership/xattrs.
pub async fn unpack(tarball: &Path, root: &Path) -> crate::Result<()> {
    crate::process::run(
        "tar",
        &[
            "--numeric-owner",
            "--xattrs-include=*.*",
            "-xpf",
            tarball.to_str().ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 tarball path")))?,
            "-C",
            root.to_str().ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 root path")))?,
        ],
    )
    .await
    .map(|_| ())
}
