//! Installing without a network: the payload on an offline live medium.
//!
//! An offline image carries, next to the live system and outside the squashfs (dracut's `dmsquash-live` mounts the medium at
//! `/run/initramfs/live`), an `offline/` directory with everything an install downloads:
//!
//! | path | what |
//! |---|---|
//! | `Packages`, `kernels/` | the kernel store (what `GENTOO_STORE_BINHOST_URL` points at) |
//! | `binhost/simple-linux/` | the project's binary host as the installer needs it: its `Packages` index cut down to what is here, the `.gpkg.tar` files, `tree.tar.zst` |
//! | `binhost/gentoo/` | the same for Gentoo's own binary host (the stage3's `[gentoo]` entry is pointed here) |
//! | `configs.git/` | the desktop configuration, a bare repository (cloned by path: `--depth 1` does not work over dumb HTTP) |
//! | `manifest` | marks the directory as a payload (and says what is in it) |
//!
//! (The stage3 is not here: the medium's `stage/` directory is where [`crate::stage3::bundled`] already looks.)
//!
//! [`activate`] serves that directory on `127.0.0.1` from a small thread and fills in the three environment variables the rest of the
//! installer already reads, unless the caller set them. Nothing else in the installer knows about it: kernels, packages, the tree
//! snapshot and the configs are fetched over HTTP like always, from the local address. The server is plain `std`; the chroot shares the
//! host's network namespace, so Portage and git inside the target reach it too.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Where the live ISO keeps the payload (`offline/` on the medium).
pub const DIRS: &[&str] = &["/run/initramfs/live/offline", "/run/live/medium/offline"];

/// The payload directory: `GENTOO_OFFLINE_DIR` if set (a test, a mounted copy), else the medium's. `GENTOO_OFFLINE_DIR=` (empty) or `0` turns
/// it off.
pub fn dir() -> Option<PathBuf> {
    match std::env::var("GENTOO_OFFLINE_DIR") {
        Ok(v) if v.trim().is_empty() || v.trim() == "0" => None,
        Ok(v) => Some(PathBuf::from(v.trim())).filter(|p| p.join("manifest").is_file()),
        Err(_) => DIRS
            .iter()
            .map(PathBuf::from)
            .find(|p| p.join("manifest").is_file()),
    }
}

static ACTIVE: OnceLock<Option<u16>> = OnceLock::new();

/// The port of the local server when this is an offline medium (started on the first call), `None` otherwise. Idempotent.
pub fn activate() -> Option<u16> {
    *ACTIVE.get_or_init(|| {
        let root = dir()?;
        let configs = format!("file://{}/configs.git", root.display());
        let listener = TcpListener::bind("127.0.0.1:0").ok()?;
        let port = listener.local_addr().ok()?.port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let root = root.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, &root);
                });
            }
        });
        let base = format!("http://127.0.0.1:{port}");
        for (key, value) in [
            ("GENTOO_STORE_BINHOST_URL", base.clone()),
            ("GENTOO_BINHOST_URL", format!("{base}/binhost/simple-linux")),
            ("GENTOO_WM_CONFIGS_URL", configs.clone()),
        ] {
            if std::env::var_os(key).is_none() {
                std::env::set_var(key, value);
            }
        }
        Some(port)
    })
}

/// Software groups whose packages are too big for the medium (LibreOffice alone is 200 MB). Offline they are not offered and an id given
/// by hand is ignored; the group can be installed on the finished system, with a network.
pub const NOT_ON_MEDIUM: &[&str] = &["office"];

/// Can this software group be installed from here? Always, except on an offline medium that left it out.
pub fn group_available(id: &str) -> bool {
    !(is_active() && NOT_ON_MEDIUM.contains(&id))
}

/// Is the install running from an offline medium (the server is up)?
pub fn is_active() -> bool {
    activate().is_some()
}

/// The URL of the local copy of Gentoo's binary host, for the `[gentoo]` entry in the target's `binrepos.conf` (the stage3's points at the
/// internet).
pub fn local_gentoo_binhost_url() -> Option<String> {
    activate().map(|p| format!("http://127.0.0.1:{p}/binhost/gentoo"))
}

/// A path from the request, mapped under `root`; `None` for anything that could leave it.
fn resolve(root: &Path, request_path: &str) -> Option<PathBuf> {
    let path = request_path.split(['?', '#']).next()?;
    let mut out = root.to_path_buf();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        if part == ".." || part == "." || part.contains('\0') || part.contains('\\') {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` against a file of `len` bytes: the inclusive range, `None` when not satisfiable.
fn byte_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if len == 0 {
        return None;
    }
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len - 1)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
        (start, end)
    };
    (start <= end && start < len).then_some((start, end))
}

fn serve(mut stream: TcpStream, root: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(60)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut parts = request.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let mut range = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_string());
            }
        }
    }
    let head_only = method == "HEAD";
    if method != "GET" && !head_only {
        return stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    let file = resolve(root, target).filter(|p| p.is_file());
    let Some(path) = file else {
        return stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let mut f = std::fs::File::open(&path)?;
    let len = f.metadata()?.len();
    let (status, start, end) = match range.as_deref().map(|r| byte_range(r, len)) {
        Some(Some((a, b))) => ("206 Partial Content", a, b),
        Some(None) => {
            return stream.write_all(
                format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes(),
            )
        }
        None => ("200 OK", 0, len.saturating_sub(1)),
    };
    let count = if len == 0 { 0 } else { end - start + 1 };
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\nContent-Length: {count}\r\nConnection: close\r\n"
    );
    if status.starts_with("206") {
        head.push_str(&format!("Content-Range: bytes {start}-{end}/{len}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if head_only || count == 0 {
        return Ok(());
    }
    f.seek(SeekFrom::Start(start))?;
    std::io::copy(&mut f.take(count), &mut stream)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_cannot_leave_the_root() {
        let root = Path::new("/srv/offline");
        assert_eq!(resolve(root, "/binhost/Packages"), Some(root.join("binhost/Packages")));
        assert_eq!(resolve(root, "/a/b?x=1"), Some(root.join("a/b")));
        assert!(resolve(root, "/../etc/passwd").is_none());
        assert!(resolve(root, "/a/../../b").is_none());
        assert!(resolve(root, "/a\\b").is_none());
    }

    #[test]
    fn byte_ranges() {
        assert_eq!(byte_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(byte_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(byte_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(byte_range("bytes=50-500", 100), Some((50, 99)));
        assert_eq!(byte_range("bytes=100-", 100), None);
        assert_eq!(byte_range("bytes=5-2", 100), None);
        assert_eq!(byte_range("items=1-2", 100), None);
    }

    #[test]
    fn serves_files_with_ranges_and_refuses_the_rest() {
        let d = std::env::temp_dir().join(format!("gentoo-installer-offline-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/f"), b"0123456789").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let root = d.clone();
        std::thread::spawn(move || {
            for s in listener.incoming().flatten().take(4) {
                let _ = serve(s, &root);
            }
        });
        let get = |req: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(req.as_bytes()).unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        let full = get("GET /sub/f HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(full.starts_with("HTTP/1.1 200") && full.ends_with("0123456789"), "{full}");
        let part = get("GET /sub/f HTTP/1.1\r\nRange: bytes=3-5\r\n\r\n");
        assert!(part.starts_with("HTTP/1.1 206") && part.ends_with("345") && part.contains("bytes 3-5/10"), "{part}");
        assert!(get("GET /nope HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
        assert!(get("GET /../x HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
        std::fs::remove_dir_all(&d).ok();
    }
}
