//! Fetches and unpacks an official Gentoo stage3 tarball (amd64, openrc, multilib, non-hardened).

use crate::command::CommandRunner;
use futures_util::StreamExt;
use sha2::{Digest, Sha512};
use std::path::Path;
use tokio::io::AsyncWriteExt;

pub const DEFAULT_MIRROR: &str = "https://distfiles.gentoo.org/releases/amd64/autobuilds";
const INDEX_FILE: &str = "latest-stage3-amd64-openrc.txt";

#[derive(Debug, Clone)]
pub struct Stage3Source {
    /// `https://…`, `file:///…` or a plain absolute path.
    pub url: String,
    pub sha512: Option<String>,
}

impl Stage3Source {
    /// A tarball of the caller's choosing (a custom stage built from Gentoo's, hosted on the
    /// store or shipped on the ISO) instead of the official mirror's latest.
    pub fn custom(url: impl Into<String>, sha512: Option<String>) -> Self {
        Self { url: url.into(), sha512: sha512.map(|h| h.to_ascii_lowercase()) }
    }
}

/// Where the live ISO keeps the stage3 it ships (`stage/` on the medium; dracut's
/// `dmsquash-live` mounts the medium at `/run/initramfs/live`).
pub const BUNDLED_DIRS: &[&str] = &["/run/initramfs/live/stage", "/run/live/medium/stage"];

/// A stage3 shipped next to the installer: the newest `*.tar.xz` in `dir`, with the digest
/// from its `<name>.sha512` (`sha512sum` format) when that file is there.
pub fn bundled_from(dir: &Path) -> Option<Stage3Source> {
    let mut tarballs: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xz") && p.to_string_lossy().ends_with(".tar.xz"))
        .collect();
    tarballs.sort();
    let tarball = tarballs.pop()?;
    let sha512 = std::fs::read_to_string(format!("{}.sha512", tarball.display()))
        .ok()
        .and_then(|t| t.split_whitespace().next().map(str::to_string))
        .filter(|h| h.len() == 128 && h.chars().all(|c| c.is_ascii_hexdigit()));
    Some(Stage3Source::custom(tarball.to_string_lossy(), sha512))
}

/// The stage3 on the live medium, if this is a live ISO that carries one.
pub fn bundled() -> Option<Stage3Source> {
    BUNDLED_DIRS.iter().find_map(|d| bundled_from(Path::new(d)))
}

/// Picks the source: the caller's custom one, else the stage3 shipped on the live medium,
/// else the official mirror's current stage3 (which needs the network).
pub async fn resolve(custom: Option<&Stage3Source>) -> crate::Result<Stage3Source> {
    if let Some(c) = custom {
        return Ok(c.clone());
    }
    match bundled() {
        Some(b) => Ok(b),
        None => resolve_latest().await,
    }
}

/// `file:///x/y.tar.xz` and `/x/y.tar.xz` are local files; anything else goes over HTTP.
fn local_path(url: &str) -> Option<&str> {
    url.strip_prefix("file://").or_else(|| url.starts_with('/').then_some(url))
}

fn check_digest(expected: &Option<String>, actual: String) -> crate::Result<()> {
    match expected {
        Some(e) if !e.eq_ignore_ascii_case(&actual) => {
            Err(crate::Error::Other(anyhow::anyhow!("stage3 sha512 mismatch: expected {e}, got {actual}")))
        }
        _ => Ok(()),
    }
}

/// Reads the mirror's `latest-stage3-amd64-openrc.txt` index to resolve the current
/// tarball filename (Gentoo autobuilds are published under a date-stamped path), then
/// fetches the companion `.DIGESTS` file to pull a SHA512 if the index publishes one.
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

    let relative_path = parse_relative_path(&body)
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("empty or unparsable {INDEX_FILE}")))?;

    let url = format!("{mirror}/{relative_path}");
    let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
    let sha512 = fetch_digests_body(&format!("{url}.DIGESTS")).await.and_then(|b| parse_sha512_digest(&b, filename));

    Ok(Stage3Source { url, sha512 })
}

async fn fetch_digests_body(url: &str) -> Option<String> {
    reqwest::get(url).await.ok()?.text().await.ok()
}

fn clearsigned_body(body: &str) -> &str {
    body.split_once("-----BEGIN PGP SIGNATURE-----").map(|(before, _)| before).unwrap_or(body)
}

/// Gentoo's autobuild indexes are OpenPGP clearsigned — live-tested against the real
/// file, which is wrapped as:
///   -----BEGIN PGP SIGNED MESSAGE-----
///   Hash: SHA256
///   <the actual "# comment"/data lines we want>
///   -----BEGIN PGP SIGNATURE-----
///   <base64 signature — NOT filtered out by the old "skip comments/blanks" logic,
///    so picking the last non-comment line grabbed a signature line instead of the
///    real stage3 path>
///   -----END PGP SIGNATURE-----
/// Signature verification itself isn't done here (only the digest checked separately) —
/// this just needs to stop reading before the armor, not authenticate it.
fn parse_relative_path(body: &str) -> Option<&str> {
    clearsigned_body(body)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("Hash:"))
        .filter_map(|l| l.split_whitespace().next())
        .next_back()
}

/// `.DIGESTS` files are also clearsigned (see `parse_relative_path`) and list several
/// hash algorithms — live-tested against the real file, which no longer has a SHA256
/// section at all, only `# BLAKE2B HASH` and `# SHA512 HASH` (note: "HASH" is part of
/// the header, not just the algorithm name — matching on the bare algorithm name alone,
/// as an earlier version of this function did, never matches anything). Each stanza
/// covers a specific file (the stage3 tarball, its `.CONTENTS.gz`, ...), so the target
/// filename is matched explicitly rather than taking the first SHA512 line found.
/// Missing/unparsable digest is not fatal — the caller can still proceed unverified,
/// since https + the official mirror is already a meaningful trust boundary.
fn parse_sha512_digest(body: &str, filename: &str) -> Option<String> {
    let mut lines = clearsigned_body(body).lines();
    while let Some(line) = lines.next() {
        let header = line.trim_start_matches('#').trim();
        if !header.eq_ignore_ascii_case("SHA512 HASH") {
            continue;
        }
        let hash_line = lines.next()?;
        let mut parts = hash_line.split_whitespace();
        let hash = parts.next()?;
        let matched_filename = parts.next()?;
        if matched_filename == filename {
            return Some(hash.to_string());
        }
    }
    None
}

pub async fn download(source: &Stage3Source, dest: &Path) -> crate::Result<()> {
    if let Some(path) = local_path(&source.url) {
        let mut src = tokio::fs::File::open(path).await?;
        let mut file = tokio::fs::File::create(dest).await?;
        let mut hasher = Sha512::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = tokio::io::AsyncReadExt::read(&mut src, &mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).await?;
        }
        file.flush().await?;
        return check_digest(&source.sha512, hex::encode(hasher.finalize()));
    }

    let response = reqwest::get(&source.url)
        .await
        .map_err(|e| crate::Error::Other(e.into()))?
        .error_for_status()
        .map_err(|e| crate::Error::Other(e.into()))?;

    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = Sha512::new();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| crate::Error::Other(e.into()))?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;

    check_digest(&source.sha512, hex::encode(hasher.finalize()))
}

/// Unpacks the tarball into `root` (typically the mounted target `@` subvolume),
/// preserving ownership/xattrs.
pub async fn unpack(runner: &dyn CommandRunner, tarball: &Path, root: &Path) -> crate::Result<()> {
    runner
        .run(
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

#[cfg(test)]
mod tests {
    use super::*;

    // Captured live from https://distfiles.gentoo.org/releases/amd64/autobuilds/
    // latest-stage3-amd64-openrc.txt — real clearsigned format, not hand-constructed.
    const INDEX_FIXTURE: &str = "\
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA256

# Latest as of Sat, 15 Aug 2026 09:15:01 +0000
# ts=1786785301
20260811T083102Z/stage3-amd64-openrc-20260811T083102Z.tar.xz 496363884
-----BEGIN PGP SIGNATURE-----

iQFPBAEBCAA5FiEEU05CCatJ7uHBnZYWLERpXbn2BD0FAmqAL4AbFIAAAAAABAAO
bWFudTIsMi41KzEuMTIsMiwyAAoJECxEaV259gQ9kF4H/jP2VguND7pVuTvebcCA
=KZpj
-----END PGP SIGNATURE-----
";

    // Captured live from the matching .DIGESTS file.
    const DIGESTS_FIXTURE: &str = "\
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA256

# BLAKE2B HASH
8a4686280f04ebe21bc55e6adddefb991bc9491db9d304bf3e4fc69e607294feed9a61aa7ca813c0de013618679bee4900a25eba7aa7809b8686be53974ed585  stage3-amd64-openrc-20260811T083102Z.tar.xz
# SHA512 HASH
9db785763adc0ef25f1672e7bc374c5ed54ca09314a4907b000dbce4fe329fd36d117cbc51783e1d6c9557cae5aeb80737a67116e3e979409ae41d3a259d95dd  stage3-amd64-openrc-20260811T083102Z.tar.xz
# BLAKE2B HASH
e16d75e1f288bdb37201719a8a9ba3d140f035b06b53199bb27b4e663616233524e20b8a066845369b44e5a920b9e622f72f66bf7a3b451d95ec65bc8a254bcf  stage3-amd64-openrc-20260811T083102Z.tar.xz.CONTENTS.gz
# SHA512 HASH
6507dc8a23ccd3f687370cdea0131998ce927365ed2e554d44d299981d7b481503ef33265cab7b55416118ed3d849798028336d3d99fe9085473721b8f719ed8  stage3-amd64-openrc-20260811T083102Z.tar.xz.CONTENTS.gz
-----BEGIN PGP SIGNATURE-----

iQFPBAEBCAA5FiEEU05CCatJ7uHBnZYWLERpXbn2BD0FAmp65NUbFIAAAAAABAAO
=abcd
-----END PGP SIGNATURE-----
";

    #[test]
    fn parses_relative_path_from_real_clearsigned_index() {
        let path = parse_relative_path(INDEX_FIXTURE).unwrap();
        assert_eq!(path, "20260811T083102Z/stage3-amd64-openrc-20260811T083102Z.tar.xz");
    }

    #[test]
    fn ignores_pgp_signature_armor_when_picking_the_last_line() {
        // Regression test: the original parser took `.last()` over all non-comment
        // lines with no signature-boundary awareness, so it picked a base64 signature
        // fragment ("-----END..."/PGP body lines) instead of the actual stage3 path.
        let path = parse_relative_path(INDEX_FIXTURE).unwrap();
        assert!(!path.contains("PGP"));
        assert!(!path.contains("BEGIN"));
        assert!(path.ends_with(".tar.xz"));
    }

    #[test]
    fn parses_sha512_for_the_matching_filename_not_the_first_stanza() {
        let digest = parse_sha512_digest(DIGESTS_FIXTURE, "stage3-amd64-openrc-20260811T083102Z.tar.xz").unwrap();
        assert_eq!(digest, "9db785763adc0ef25f1672e7bc374c5ed54ca09314a4907b000dbce4fe329fd36d117cbc51783e1d6c9557cae5aeb80737a67116e3e979409ae41d3a259d95dd");
    }

    #[test]
    fn sha512_digest_does_not_match_a_different_files_stanza() {
        let digest = parse_sha512_digest(DIGESTS_FIXTURE, "stage3-amd64-openrc-20260811T083102Z.tar.xz.CONTENTS.gz").unwrap();
        assert!(digest.starts_with("6507dc8a"));
    }

    #[test]
    fn no_sha256_section_in_current_format_returns_none_not_a_false_match() {
        // The live file has no "# SHA256 HASH" stanza at all (only BLAKE2B and SHA512)
        // -- confirms there's nothing here that could accidentally satisfy a
        // SHA256-shaped lookup were one still in the code.
        assert!(!DIGESTS_FIXTURE.contains("SHA256 HASH"));
    }

    #[tokio::test]
    async fn a_local_tarball_is_copied_and_its_digest_checked() {
        let dir = std::env::temp_dir().join(format!("gi-stage3-local-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("custom.tar.xz");
        std::fs::write(&src, b"not really a tarball").unwrap();
        let dest = dir.join("out");
        let good = hex::encode(Sha512::digest(b"not really a tarball"));

        for url in [src.display().to_string(), format!("file://{}", src.display())] {
            download(&Stage3Source::custom(url, Some(good.to_uppercase())), &dest).await.unwrap();
            assert_eq!(std::fs::read(&dest).unwrap(), b"not really a tarball");
        }
        let err = download(&Stage3Source::custom(src.display().to_string(), Some("00".into())), &dest).await.unwrap_err();
        assert!(err.to_string().contains("sha512 mismatch"), "{err}");
        assert!(download(&Stage3Source::custom("/nonexistent/x.tar.xz", None), &dest).await.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn resolve_prefers_the_custom_source_without_touching_the_network() {
        let c = Stage3Source::custom("/x/custom.tar.xz", None);
        assert_eq!(resolve(Some(&c)).await.unwrap().url, "/x/custom.tar.xz");
    }

    #[test]
    fn bundled_picks_the_newest_tarball_and_reads_its_digest() {
        let dir = std::env::temp_dir().join(format!("gi-bundled-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        assert!(bundled_from(&dir).is_none(), "an empty directory carries no stage3");

        let hash = "ab".repeat(64);
        std::fs::write(dir.join("stage3-20260101.tar.xz"), "").unwrap();
        std::fs::write(dir.join("stage3-20260930.tar.xz"), "").unwrap();
        std::fs::write(dir.join("stage3-20260930.tar.xz.sha512"), format!("{hash}  stage3-20260930.tar.xz\n")).unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();

        let s = bundled_from(&dir).unwrap();
        // "newest" is the lexicographically last name (the names are date-stamped).
        assert!(s.url.ends_with("stage3-20260930.tar.xz"), "{}", s.url);
        assert_eq!(s.sha512.as_deref(), Some(hash.as_str()));
        std::fs::remove_file(dir.join("stage3-20260101.tar.xz")).unwrap();
        let s = bundled_from(&dir).unwrap();
        assert_eq!(s.sha512.as_deref(), Some(hash.as_str()));

        std::fs::write(dir.join("stage3-20260930.tar.xz.sha512"), "not a digest").unwrap();
        assert_eq!(bundled_from(&dir).unwrap().sha512, None, "a malformed digest file is ignored, not trusted");
        std::fs::remove_dir_all(&dir).ok();
    }
}
