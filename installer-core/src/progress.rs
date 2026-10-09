//! Reading progress out of what the install's commands print, and out of the kernel while they run.

use std::path::Path;

/// `>>> Emerging (3 of 31) gui-wm/niri-26.04::guru` -> `(3, 31, "gui-wm/niri-26.04")`. Portage prints this line when it
/// starts each package of a merge list (also `>>> Emerging binary (3 of 31) ...`).
pub fn emerge_position(line: &str) -> Option<(u64, u64, String)> {
    let rest = line.trim_start().strip_prefix(">>>")?.trim_start();
    let rest = rest.strip_prefix("Emerging")?.trim_start();
    let rest = rest.strip_prefix("binary").map_or(rest, str::trim_start);
    let inner = rest.strip_prefix('(')?;
    let (counts, after) = inner.split_once(')')?;
    let (n, m) = counts.split_once(" of ")?;
    let package = after
        .split_whitespace()
        .next()
        .unwrap_or("")
        .split("::")
        .next()
        .unwrap_or("")
        .to_string();
    Some((n.trim().parse().ok()?, m.trim().parse().ok()?, package))
}

/// How far a process that is reading `path` has got, in bytes: the offset of its open file descriptor. `tar -xpf big.tar.xz`
/// has no progress output of its own, but the offset into the compressed file is an exact progress meter. `None` when
/// no process has the file open (not started yet, or already finished).
pub fn open_file_offset(path: &Path) -> Option<u64> {
    let want = std::fs::canonicalize(path).ok()?;
    for proc_entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let name = proc_entry.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(proc_entry.path().join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            if std::fs::read_link(fd.path()).ok().as_deref() != Some(want.as_path()) {
                continue;
            }
            let info = proc_entry.path().join("fdinfo").join(fd.file_name());
            if let Some(pos) = std::fs::read_to_string(info).ok().and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("pos:").map(|v| v.trim().parse().ok()))?
            }) {
                return Some(pos);
            }
        }
    }
    None
}

/// Megabytes with one decimal below 10, whole above: `7.4 MB`, `240 MB`.
pub fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb < 10.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{mb:.0} MB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_emerge_counter_is_read_from_the_start_line() {
        assert_eq!(
            emerge_position(">>> Emerging (3 of 31) gui-wm/niri-26.04::guru"),
            Some((3, 31, "gui-wm/niri-26.04".into()))
        );
        assert_eq!(
            emerge_position(">>> Emerging binary (12 of 140) dev-lang/rust-1.90::gentoo"),
            Some((12, 140, "dev-lang/rust-1.90".into()))
        );
        assert_eq!(
            emerge_position(">>> Installing (3 of 31) gui-wm/niri-26.04"),
            None
        );
        assert_eq!(emerge_position("random output (3 of 31)"), None);
    }

    #[test]
    fn a_file_we_hold_open_reports_its_offset() {
        use std::io::{Read, Write};
        let p = std::env::temp_dir().join(format!("gi-fdpos-{}", std::process::id()));
        std::fs::File::create(&p)
            .unwrap()
            .write_all(&[7u8; 5000])
            .unwrap();
        let mut f = std::fs::File::open(&p).unwrap();
        let mut buf = [0u8; 1234];
        f.read_exact(&mut buf).unwrap();
        assert_eq!(open_file_offset(&p), Some(1234));
        drop(f);
        assert_eq!(open_file_offset(&p), None);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(megabytes(7_759_462), "7.4 MB");
        assert_eq!(megabytes(251_658_240), "240 MB");
    }
}
