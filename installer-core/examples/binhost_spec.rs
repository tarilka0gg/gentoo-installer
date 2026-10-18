//! `cargo run -p installer-core --example binhost_spec -- <dir> [all]` writes the Portage configuration of the software the installer
//! installs by default (what `packages::install` writes to the target): `world` additions, `package.use`,
//! `package.accept_keywords` and `package.license`. The binhost build (`iso/binhost/`) uses these so that the packages it
//! builds have exactly the USE flags the installer will ask for; a binary with other flags is ignored and compiled again.
//! The data is the profiles' (`profiles/`): the flavor `desktop` with its default mix-ins, or with every software mix-in (`all`).
use installer_core::packages;
use slp_profiles::{Repo, Selection};

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    // `all` as the second argument: every group, not just the ticked-by-default ones (the GUI offers them all)
    let ids: Vec<String> = if std::env::args().nth(2).as_deref() == Some("all") {
        // not the groups from other overlays (Steam, PineconeMC, the Craft apps): there is nothing to prebuild for them, the binhost
        // build has no such overlay
        let repo = Repo::bundled();
        packages::GROUPS
            .iter()
            .filter(|g| repo.info(slp_profiles::Kind::MixIn, g.id).is_some_and(|i| i.overlays.is_empty()))
            .map(|g| g.id.to_string())
            .collect()
    } else {
        packages::default_ids()
    };
    let selection = Selection { flavor: packages::DEFAULT_FLAVOR.to_string(), mix_ins: ids };
    let effective = Repo::bundled().resolve(&selection).expect("profile");
    for (path, content) in effective.binhost_spec() {
        let file = dir.join(&path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }
}
