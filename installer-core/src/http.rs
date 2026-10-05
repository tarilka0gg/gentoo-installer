//! Tiny shared HTTP download helper. `stage3.rs` keeps its own streaming+hashing variant
//! (different requirements — progress-relevant size, SHA256 verification); this is for
//! callers that just need "fetch this URL to this path" with no extra bookkeeping.

use std::path::Path;
use tokio::io::AsyncWriteExt;

pub async fn download_to_file(url: &str, dest: &Path) -> crate::Result<()> {
    let mut response = reqwest::get(url)
        .await
        .map_err(|e| crate::Error::Other(e.into()))?
        .error_for_status()
        .map_err(|e| crate::Error::Other(e.into()))?;

    let mut file = tokio::fs::File::create(dest).await?;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| crate::Error::Other(e.into()))?
    {
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(())
}
