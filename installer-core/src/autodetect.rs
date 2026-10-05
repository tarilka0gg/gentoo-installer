//! Language, locale and time zone guessed from what the machine already tells us.
//!
//! No network geolocation (see `detect::DetectedSystem::timezone`): the keyboard layout the user booted
//! with is the strongest local hint, and it names a country, which names a time zone and a locale.
//! Everything here is a *default*; the installer still lets the user override each value.

use crate::locale::DEFAULT_LOCALE;
use std::path::Path;

/// `(layout prefix, locale, ISO 3166 country)`. The layout is the console keymap's name (`ua`, `de-latin1`,
/// `pl2`, …); only its leading letters are compared.
const LAYOUTS: &[(&str, &str, &str)] = &[
    ("ua", "uk_UA.UTF-8", "UA"),
    ("ru", "ru_RU.UTF-8", "RU"),
    ("by", "be_BY.UTF-8", "BY"),
    ("pl", "pl_PL.UTF-8", "PL"),
    ("de", "de_DE.UTF-8", "DE"),
    ("fr", "fr_FR.UTF-8", "FR"),
    ("es", "es_ES.UTF-8", "ES"),
    ("it", "it_IT.UTF-8", "IT"),
    ("pt", "pt_PT.UTF-8", "PT"),
    ("br", "pt_BR.UTF-8", "BR"),
    ("cz", "cs_CZ.UTF-8", "CZ"),
    ("sk", "sk_SK.UTF-8", "SK"),
    ("hu", "hu_HU.UTF-8", "HU"),
    ("ro", "ro_RO.UTF-8", "RO"),
    ("bg", "bg_BG.UTF-8", "BG"),
    ("nl", "nl_NL.UTF-8", "NL"),
    ("se", "sv_SE.UTF-8", "SE"),
    ("no", "nb_NO.UTF-8", "NO"),
    ("dk", "da_DK.UTF-8", "DK"),
    ("fi", "fi_FI.UTF-8", "FI"),
    ("tr", "tr_TR.UTF-8", "TR"),
    ("gr", "el_GR.UTF-8", "GR"),
    ("jp", "ja_JP.UTF-8", "JP"),
    ("uk", "en_GB.UTF-8", "GB"),
    ("gb", "en_GB.UTF-8", "GB"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guess {
    /// `locale-gen` names, the system language first; always ends with `en_US.UTF-8` when it is not the main one.
    pub locales: Vec<String>,
    /// IANA zone, `None` when nothing pointed at one unambiguously.
    pub timezone: Option<String>,
}

fn entry(layout: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    let l = layout.trim().to_ascii_lowercase();
    // `ua`, `ua-utf`, `ua_utf`; `uk` is the *British* layout on Linux consoles, so it is matched before `ua`-style prefixes.
    LAYOUTS.iter().find(|(p, _, _)| {
        l == *p
            || l.starts_with(&format!("{p}-"))
            || l.starts_with(&format!("{p}_"))
            || (p.len() == 2
                && l.starts_with(p)
                && l.len() <= 4
                && l[2..].chars().all(|c| c.is_ascii_digit() || c == 'l'))
    })
}

/// The locales for a keyboard layout: its language, then `en_US.UTF-8` (tools and docs expect it).
pub fn locales_for_layout(layout: &str) -> Vec<String> {
    match entry(layout) {
        Some((_, locale, _)) => vec![locale.to_string(), DEFAULT_LOCALE.to_string()],
        None => vec![DEFAULT_LOCALE.to_string()],
    }
}

/// The one time zone of a country, from `zone.tab`'s text: the only row, or the row described as
/// "most of …". `None` for countries with several equally good zones (a wrong guess is worse than UTC).
pub fn zone_for_country(zone_tab: &str, country: &str) -> Option<String> {
    let rows: Vec<Vec<&str>> = zone_tab
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .filter(|f| f.len() >= 3 && f[0] == country)
        .collect();
    match rows.as_slice() {
        [only] => Some(only[2].to_string()),
        many => many
            .iter()
            .find(|f| f.get(3).is_some_and(|c| c.starts_with("most of")))
            .map(|f| f[2].to_string()),
    }
}

/// Guess from the keyboard layout and the zone the live system already runs in. A live zone other than UTC
/// wins (someone set it on purpose); UTC is what a freshly booted ISO has, which says nothing.
pub fn guess(layout: &str, live_zone: Option<&str>, zone_tab: &Path) -> Guess {
    let locales = locales_for_layout(layout);
    let live = live_zone.filter(|z| !z.is_empty() && *z != "UTC" && *z != "Etc/UTC");
    let timezone = live.map(str::to_string).or_else(|| {
        let (_, _, country) = entry(layout)?;
        zone_for_country(&std::fs::read_to_string(zone_tab).ok()?, country)
    });
    Guess { locales, timezone }
}

/// [`guess`] for `layout` on the running live system.
pub fn guess_for_layout(layout: &str) -> Guess {
    guess(
        layout,
        crate::timezone::detect_current().as_deref(),
        Path::new("/usr/share/zoneinfo/zone.tab"),
    )
}

/// [`guess_for_layout`] with the live console's own layout.
pub fn guess_here() -> Guess {
    guess_for_layout(&crate::keyboard::detect_current())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAB: &str = "# comment\nDE\t+5230+01322\tEurope/Berlin\tmost of Germany\nDE\t+4742+00841\tEurope/Busingen\tBusingen\nUA\t+4457+03406\tEurope/Simferopol\tCrimea\nUA\t+5026+03031\tEurope/Kyiv\tmost of Ukraine\nUS\t+404251-0740023\tAmerica/New_York\tEastern (most areas)\nUS\t+415100-0873900\tAmerica/Chicago\tCentral (most areas)\nPL\t+5215+02100\tEurope/Warsaw\t\n";

    #[test]
    fn a_ukrainian_keyboard_means_ukrainian_with_english_alongside() {
        assert_eq!(locales_for_layout("ua"), ["uk_UA.UTF-8", "en_US.UTF-8"]);
        assert_eq!(locales_for_layout("ua-utf"), ["uk_UA.UTF-8", "en_US.UTF-8"]);
        assert_eq!(
            locales_for_layout("de-latin1"),
            ["de_DE.UTF-8", "en_US.UTF-8"]
        );
        assert_eq!(locales_for_layout("pl2"), ["pl_PL.UTF-8", "en_US.UTF-8"]);
    }

    #[test]
    fn unknown_or_us_layouts_stay_english() {
        assert_eq!(locales_for_layout("us"), ["en_US.UTF-8"]);
        assert_eq!(locales_for_layout(""), ["en_US.UTF-8"]);
        assert_eq!(
            locales_for_layout("dvorak"),
            ["en_US.UTF-8"],
            "`dvorak` is not `dk`"
        );
    }

    #[test]
    fn a_country_with_one_zone_or_a_main_zone_gets_it_and_an_ambiguous_one_gets_none() {
        assert_eq!(
            zone_for_country(TAB, "PL").as_deref(),
            Some("Europe/Warsaw")
        );
        assert_eq!(
            zone_for_country(TAB, "UA").as_deref(),
            Some("Europe/Kyiv"),
            "not Crimea, which is listed first"
        );
        assert_eq!(
            zone_for_country(TAB, "DE").as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(zone_for_country(TAB, "US"), None);
        assert_eq!(zone_for_country(TAB, "XX"), None);
    }

    #[test]
    fn a_live_zone_set_on_purpose_beats_the_keyboard_but_utc_does_not() {
        let dir = std::env::temp_dir().join(format!(
            "gentoo-installer-autodetect-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let tab = dir.join("zone.tab");
        std::fs::write(&tab, TAB).unwrap();
        assert_eq!(
            guess("ua", Some("UTC"), &tab).timezone.as_deref(),
            Some("Europe/Kyiv")
        );
        assert_eq!(
            guess("ua", None, &tab).timezone.as_deref(),
            Some("Europe/Kyiv")
        );
        assert_eq!(
            guess("ua", Some("Europe/Berlin"), &tab).timezone.as_deref(),
            Some("Europe/Berlin")
        );
        assert_eq!(guess("us", Some("UTC"), &tab).timezone, None);
        assert_eq!(guess("ua", None, &dir.join("missing")).timezone, None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
