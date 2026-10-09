//! `cargo run -p installer-core --example binhost_spec -- <dir>` writes the Portage configuration of the groups the
//! installer installs by default (what `packages::install` writes to the target): `world` additions, `package.use`,
//! `package.accept_keywords` and `package.license`. The binhost build (`iso/binhost/`) uses these so that the packages it
//! builds have exactly the USE flags the installer will ask for; a binary with other flags is ignored and compiled again.
use installer_core::packages;
use std::fmt::Write as _;

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    let groups = packages::resolve(&packages::default_ids()).expect("default groups");
    let (mut world, mut keywords, mut uses, mut licenses) =
        (String::new(), String::new(), String::new(), String::new());
    for g in &groups {
        for a in g.atoms {
            if !world.lines().any(|l| l == *a) {
                writeln!(world, "{a}").unwrap();
            }
        }
        for a in g.testing {
            writeln!(keywords, "{a} ~amd64").unwrap();
        }
        for l in g.use_flags {
            writeln!(uses, "{l}").unwrap();
        }
        for l in g.licenses {
            writeln!(licenses, "{l}").unwrap();
        }
    }
    for sub in ["package.use", "package.accept_keywords", "package.license"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    std::fs::write(dir.join("world-groups"), world).unwrap();
    std::fs::write(
        dir.join("package.accept_keywords/gentoo-installer-packages"),
        keywords,
    )
    .unwrap();
    std::fs::write(dir.join("package.use/gentoo-installer-packages"), uses).unwrap();
    std::fs::write(
        dir.join("package.license/gentoo-installer-packages"),
        licenses,
    )
    .unwrap();
}
