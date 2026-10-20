//! The profile data against what the installer did before it existed: the old built-in package table
//! (`golden/groups.json`) and the binhost spec it generated (`golden/spec-*`). Both were captured from the code this data
//! replaced, so a difference here is a change in what an install does.
use slp_profiles::{Kind, Repo, Selection};

const GROUPS: &str = include_str!("golden/groups.json");

fn strs(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn every_old_group_is_a_mix_in_with_the_same_contents_in_the_same_order() {
    let repo = Repo::bundled();
    let old: Vec<serde_json::Value> = serde_json::from_str(GROUPS).unwrap();
    // the groups that came later, from other overlays, are not in the old table
    let software: Vec<_> = repo
        .software()
        .into_iter()
        .filter(|m| m.overlays.is_empty())
        .collect();
    assert_eq!(
        software.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        old.iter()
            .map(|g| g["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        "the software mix-ins and their order"
    );
    let defaults = repo.info(Kind::Flavor, "desktop").unwrap().defaults;
    for (m, g) in software.iter().zip(&old) {
        let id = g["id"].as_str().unwrap();
        assert_eq!(m.name, g["name"].as_str().unwrap(), "{id} name");
        assert_eq!(
            m.description,
            g["description"].as_str().unwrap(),
            "{id} description"
        );
        assert_eq!(m.atoms, strs(&g["atoms"]), "{id} atoms");
        assert_eq!(m.testing, strs(&g["testing"]), "{id} testing");
        assert_eq!(m.use_lines, strs(&g["use_flags"]), "{id} use");
        assert_eq!(m.services, strs(&g["services"]), "{id} services");
        assert_eq!(m.licenses, strs(&g["licenses"]), "{id} licenses");
        assert_eq!(m.guru, g["guru"].as_bool().unwrap(), "{id} guru");
        assert_eq!(
            defaults.iter().any(|d| d == id),
            g["default"].as_bool().unwrap(),
            "{id} ticked by default"
        );
    }
}

/// The old spec repeated a line when two groups asked for the same USE flag (`x11-libs/gtk+ X` from Files and from the disk tools);
/// each line once says the same to Portage, so a repeated line is not a difference.
fn without_repeats(text: &str) -> String {
    let mut seen = Vec::new();
    for l in text.lines() {
        if !seen.contains(&l) {
            seen.push(l);
        }
    }
    seen.iter().map(|l| format!("{l}\n")).collect()
}

fn spec_matches(mix_ins: Vec<String>, golden: &str) {
    let eff = Repo::bundled()
        .resolve(&Selection {
            flavor: "desktop".into(),
            mix_ins,
        })
        .unwrap();
    for (path, content) in eff.binhost_spec() {
        let expected = std::fs::read_to_string(format!(
            "{}/tests/golden/{golden}/{path}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert_eq!(content, without_repeats(&expected), "{golden}/{path}");
    }
}

#[test]
fn the_binhost_spec_of_the_default_install_is_what_it_was() {
    let defaults = Repo::bundled()
        .info(Kind::Flavor, "desktop")
        .unwrap()
        .defaults;
    spec_matches(defaults, "spec-default");
}

#[test]
fn the_binhost_spec_of_every_group_is_what_it_was() {
    let all = Repo::bundled()
        .software()
        .into_iter()
        .filter(|m| m.overlays.is_empty())
        .map(|m| m.id)
        .collect();
    spec_matches(all, "spec-all");
}

#[test]
fn the_flavors_offered_are_the_three_and_the_live_one_is_hidden() {
    let repo = Repo::bundled();
    let shown: Vec<String> = repo
        .flavors()
        .into_iter()
        .filter(|f| !f.hidden)
        .map(|f| f.id)
        .collect();
    assert_eq!(shown, vec!["minimal", "desktop", "workstation"]);
    let ws = repo.info(Kind::Flavor, "workstation").unwrap();
    assert!(
        ws.defaults.contains(&"dev".to_string()) && ws.defaults.contains(&"terminal".to_string())
    );
    assert!(!repo
        .info(Kind::Flavor, "minimal")
        .unwrap()
        .defaults
        .contains(&"terminal".to_string()));
    assert_eq!(
        repo.resolve(&Selection {
            flavor: "minimal".into(),
            mix_ins: vec![]
        })
        .unwrap()
        .desktop,
        false
    );
    assert_eq!(
        repo.resolve(&Selection {
            flavor: "workstation".into(),
            mix_ins: vec![]
        })
        .unwrap()
        .desktop,
        true
    );
}

#[test]
fn the_graphics_mix_ins_name_the_drivers_the_installer_picks() {
    let repo = Repo::bundled();
    let vc = |id: &str| {
        repo.resolve(&Selection {
            flavor: "desktop".into(),
            mix_ins: vec![id.into()],
        })
        .unwrap()
        .make_conf
    };
    assert_eq!(
        vc("gfx-amd"),
        vec![(
            "VIDEO_CARDS".to_string(),
            "amdgpu radeonsi".to_string(),
            true
        )]
    );
    assert_eq!(vc("gfx-legacy-radeon")[0].1, "radeon r300 r600");
    assert_eq!(vc("gfx-intel")[0].1, "intel");
    assert_eq!(vc("gfx-virtual")[0].1, "virgl");
    // a hybrid laptop: both, and the alternatives for one vendor exclude each other
    let both = repo
        .resolve(&Selection {
            flavor: "desktop".into(),
            mix_ins: vec!["gfx-nvidia".into(), "gfx-intel".into()],
        })
        .unwrap();
    assert_eq!(
        both.make_conf,
        vec![("VIDEO_CARDS".to_string(), "nvidia intel".to_string(), true)]
    );
    assert!(repo
        .resolve(&Selection {
            flavor: "desktop".into(),
            mix_ins: vec!["gfx-nvidia".into(), "gfx-nouveau".into()]
        })
        .is_err());
    assert!(repo.software().iter().all(|m| !m.id.starts_with("gfx-")));
}

#[test]
fn every_package_from_the_local_overlay_has_an_ebuild_and_a_manifest() {
    let repo = Repo::bundled();
    let files = repo.overlay_files();
    assert!(files.contains_key("profiles/repo_name") && files.contains_key("metadata/layout.conf"));
    let from_overlay: Vec<_> = repo
        .software()
        .into_iter()
        .filter(|m| m.overlays.iter().any(|o| o == "slp-local"))
        .collect();
    assert!(from_overlay.len() >= 3, "minecraft, creative, craft-office");
    for m in from_overlay {
        for atom in &m.atoms {
            let dir = format!("{atom}/");
            assert!(
                files
                    .keys()
                    .any(|k| k.starts_with(&dir) && k.ends_with(".ebuild")),
                "{atom} has no ebuild in the overlay"
            );
            assert!(
                files[&format!("{dir}Manifest")].contains("DIST "),
                "{atom}: the Manifest pins no archive"
            );
        }
    }
}

#[test]
fn choosing_steam_and_the_craft_apps_writes_their_overlays_and_keywords() {
    let repo = Repo::bundled();
    let sel = Selection {
        flavor: "desktop".into(),
        mix_ins: vec!["steam".into(), "creative".into()],
    };
    let eff = repo.resolve(&sel).unwrap();
    assert_eq!(eff.overlays, ["steam-overlay", "slp-local"]);
    let root = std::env::temp_dir().join(format!("slp-overlay-test-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    slp_profiles::render(&root, &sel, &eff, Default::default()).unwrap();
    let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap();
    let keywords = read("etc/portage/package.accept_keywords/slp");
    assert!(
        keywords.contains("*/*::steam-overlay ~amd64\n")
            && keywords.contains("*/*::slp-local ~amd64\n"),
        "{keywords}"
    );
    assert!(read("etc/portage/package.license/slp")
        .contains("games-util/steam-launcher ValveSteamLicense"));
    assert!(read("etc/portage/repos.conf/slp-local.conf")
        .contains("location = /var/db/repos/slp-local"));
    assert!(
        read("var/db/repos/slp-local/media-gfx/photocraft-bin/Manifest").contains("photocraft-")
    );
    assert_eq!(
        read("var/db/repos/slp-local/profiles/repo_name").trim(),
        "slp-local"
    );
    // a second run changes nothing in the overlay
    let again = slp_profiles::write_local_overlay(&root, repo.overlay_files()).unwrap();
    assert!(again.is_empty(), "{again:?}");
    // a choice without them writes neither
    let plain = repo
        .resolve(&Selection {
            flavor: "desktop".into(),
            mix_ins: vec!["audio".into()],
        })
        .unwrap();
    assert!(plain.overlays.is_empty());
    std::fs::remove_dir_all(&root).ok();
}
