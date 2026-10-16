//! Time zone: detection + list, for the installer's optional "Advanced setup" manual
//! override (auto-detected by default — same detection `detect::gather` already does).

use std::path::Path;

const ZONEINFO_DIR: &str = "/usr/share/zoneinfo";

/// `Region/City` zone names, walked straight off the live medium's own zoneinfo data
/// rather than a bundled list — it's already correct and already there. Skips the
/// `posix/`/`right/` alias trees and non-zone files (`posixrules`, `Factory`, the
/// top-level `*.tab` metadata files) so the list is just real, selectable zones.
pub fn list_zones() -> Vec<String> {
    let mut zones = Vec::new();
    walk(Path::new(ZONEINFO_DIR), "", &mut zones);
    zones.sort();
    zones
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    const SKIP_DIRS: &[&str] = &["posix", "right"];
    const SKIP_FILES: &[&str] = &["posixrules", "Factory", "localtime"];

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('+') || name.ends_with(".tab") || name.ends_with(".list") {
            continue;
        }
        let path = entry.path();
        let qualified = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };

        if path.is_dir() {
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            walk(&path, &qualified, out);
        } else if !SKIP_FILES.contains(&name) {
            out.push(qualified);
        }
    }
}

/// One selectable zone with what a person would search for: its country and the place the table names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    /// `Europe/Kyiv`
    pub id: String,
    /// `Ukraine`; empty for zones that belong to no country (`UTC`, `Etc/GMT+3`)
    pub country: String,
    /// The table's remark: `most of Ukraine`, `Crimea`; empty when it has none
    pub note: String,
}

impl Zone {
    /// `Kyiv` for `Europe/Kyiv`, `Buenos Aires` for `America/Argentina/Buenos_Aires`.
    pub fn city(&self) -> String {
        self.id
            .rsplit('/')
            .next()
            .unwrap_or(&self.id)
            .replace('_', " ")
    }

    /// Whether every word of `query` appears in the id, the city, the country, the remark or a well-known older name
    /// of the place (case does not matter). An empty query matches everything.
    pub fn matches(&self, query: &str) -> bool {
        let haystack = format!(
            "{} {} {} {} {}",
            self.id.replace('_', " "),
            self.city(),
            self.country,
            self.note,
            old_names(&self.id)
        )
        .to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|word| haystack.contains(word))
    }
}

/// Names people still type for places whose zone was renamed.
fn old_names(id: &str) -> &'static str {
    match id {
        "Europe/Kyiv" => "Kiev",
        "Asia/Kolkata" => "Calcutta Delhi Mumbai Bombay",
        "Asia/Ho_Chi_Minh" => "Saigon",
        "Asia/Yangon" => "Rangoon",
        "America/Argentina/Buenos_Aires" => "Buenos Aires",
        "Atlantic/Faroe" => "Faeroe",
        "Pacific/Kanton" => "Enderbury Canton",
        "Europe/Chisinau" => "Kishinev",
        "Asia/Kathmandu" => "Katmandu",
        _ => "",
    }
}

/// The zones of `zone.tab` with their countries (`iso3166.tab`), sorted by country and then by name; the plain
/// walk of the zoneinfo directory when the tables are not there. `UTC` is always offered.
pub fn list_zone_details() -> Vec<Zone> {
    let tab = std::fs::read_to_string(format!("{ZONEINFO_DIR}/zone.tab")).unwrap_or_default();
    let names = std::fs::read_to_string(format!("{ZONEINFO_DIR}/iso3166.tab")).unwrap_or_default();
    let mut zones = parse_zone_tab(&tab, &names);
    if zones.is_empty() {
        zones = list_zones()
            .into_iter()
            .map(|id| Zone {
                id,
                country: String::new(),
                note: String::new(),
            })
            .collect();
    }
    if !zones.iter().any(|z| z.id == "UTC") {
        zones.push(Zone {
            id: "UTC".into(),
            country: String::new(),
            note: "Coordinated Universal Time".into(),
        });
    }
    zones.sort_by(|a, b| (&a.country, &a.id).cmp(&(&b.country, &b.id)));
    zones
}

fn parse_zone_tab(zone_tab: &str, iso3166: &str) -> Vec<Zone> {
    let country_names: std::collections::HashMap<&str, &str> = iso3166
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .collect();
    zone_tab
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 3).then(|| Zone {
                id: f[2].to_string(),
                country: country_names.get(f[0]).copied().unwrap_or(f[0]).to_string(),
                note: f.get(3).copied().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// The zone the internet connection says this computer is in (one request to ipapi.co, made only when asked for).
/// `None` when there is no network or the answer is not a zone of this system.
pub fn detect_by_ip() -> Option<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let text = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .ok()?;
        client
            .get("https://ipapi.co/timezone/")
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .text()
            .await
            .ok()
    })?;
    let zone = text.trim().to_string();
    list_zone_details()
        .iter()
        .any(|z| z.id == zone)
        .then_some(zone)
}

/// Live-environment detection: `/etc/timezone` if present, else the `/etc/localtime`
/// symlink target. `None` if neither resolves — caller defaults to UTC.
pub fn detect_current() -> Option<String> {
    std::fs::read_to_string("/etc/timezone")
        .ok()
        .map(|s| s.trim().to_string())
        .or_else(|| {
            std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                p.strip_prefix("/usr/share/zoneinfo/")
                    .ok()
                    .map(|p| p.display().to_string())
            })
        })
}

/// Writes `/etc/timezone` and symlinks `/etc/localtime` in the target — the two files
/// Gentoo's `sys-libs/timezone-data` expects, both plain filesystem operations that need
/// no chroot.
pub async fn apply(target: &Path, zone: &str) -> crate::Result<()> {
    tokio::fs::create_dir_all(target.join("etc")).await?;
    tokio::fs::write(target.join("etc/timezone"), format!("{zone}\n")).await?;

    let localtime = target.join("etc/localtime");
    tokio::fs::remove_file(&localtime).await.ok();
    let relative = format!("../usr/share/zoneinfo/{zone}");
    tokio::fs::symlink(&relative, &localtime).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_are_found_by_city_country_and_old_name() {
        let zones = parse_zone_tab(
            "UA\t+5026+03031\tEurope/Kyiv\tmost of Ukraine\nAR\t-3436-05827\tAmerica/Argentina/Buenos_Aires\tBuenos Aires (BA, CF)\n",
            "UA\tUkraine\nAR\tArgentina\n",
        );
        let kyiv = &zones[0];
        assert_eq!(
            (kyiv.id.as_str(), kyiv.country.as_str()),
            ("Europe/Kyiv", "Ukraine")
        );
        for q in ["kyiv", "Kiev", "ukraine", "europe kyiv", ""] {
            assert!(kyiv.matches(q), "{q}");
        }
        assert!(!kyiv.matches("paris"));
        assert!(zones[1].matches("buenos aires") && zones[1].matches("argentina"));
        assert_eq!(zones[1].city(), "Buenos Aires");
    }

    #[test]
    fn lists_known_zones_from_this_machine() {
        let zones = list_zones();
        assert!(
            !zones.is_empty(),
            "expected {ZONEINFO_DIR} to be readable on this dev machine"
        );
        assert!(zones.iter().any(|z| z == "Europe/Kyiv"));
    }

    #[test]
    fn skips_alias_trees_and_metadata_files() {
        let zones = list_zones();
        assert!(!zones
            .iter()
            .any(|z| z.starts_with("posix/") || z.starts_with("right/")));
        assert!(!zones.iter().any(|z| z == "zone.tab" || z == "iso3166.tab"));
    }
}
