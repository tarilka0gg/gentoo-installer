//! Simple Linux profiles: **flavors** and **mix-ins**.
//!
//! A system is one flavor (what kind of system: `minimal`, `desktop`, `workstation`; a flavor inherits from another one) plus any
//! number of mix-ins (optional bundles of packages and the USE flags they need; the `gfx-*` ones are the graphics card, one at a
//! time). The data is the `profiles/` directory of the repository, compiled into this crate ([`BUNDLE`]); `profiles/README.md`
//! describes the files. This crate reads it ([`Repo`]), works out what a choice means ([`Repo::resolve`] → [`Effective`]) and writes
//! it as Portage configuration ([`render`]). The installer and the `slp` command both use it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

/// The marker the files written by [`render`] start with.
pub const MANAGED: &str = "# Written by slp (Simple Linux profiles); change the profile with `slp`, not this file.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Flavor,
    MixIn,
}

impl Kind {
    pub fn dir(self) -> &'static str {
        match self {
            Kind::Flavor => "flavor",
            Kind::MixIn => "mix-in",
        }
    }
}

/// What a flavor or mix-in is, by its own files (not including what it inherits).
#[derive(Debug, Clone)]
pub struct Info {
    /// `audio`, `desktop`: the name without the kind directory.
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub description: String,
    /// Mix-ins with the same group exclude each other (`gfx`).
    pub group: Option<String>,
    /// A package of this profile comes from the GURU overlay.
    pub guru: bool,
    pub needs_desktop: bool,
    /// A graphics card (`gfx-*`): offered with the graphics, not as software to tick.
    pub graphics: bool,
    /// Flavors: whether the system gets a desktop (`None`: inherits).
    pub desktop: Option<bool>,
    pub hidden: bool,
    /// Parents as full ids (`flavor/minimal`).
    pub parents: Vec<String>,
    /// Flavors: the mix-ins ticked by default by this flavor and everything it inherits, in the order of `mix-in/index`.
    pub defaults: Vec<String>,
    /// Packages, without the `~` marker.
    pub atoms: Vec<String>,
    /// The packages among `atoms` that only have a `~amd64` keyword.
    pub testing: Vec<String>,
    pub use_lines: Vec<String>,
    pub licenses: Vec<String>,
    pub services: Vec<String>,
    /// `(key, value, append)`: `append` is `KEY+="value"`, which adds words to the other profiles' value.
    pub make_conf: Vec<(String, String, bool)>,
    pub rebuild: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Unknown { kind: &'static str, id: String, known: Vec<String> },
    Conflict { group: String, a: String, b: String },
    Cycle(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unknown { kind, id, known } => write!(f, "unknown {kind} {id:?}; known: {}", known.join(", ")),
            Error::Conflict { group, a, b } => {
                write!(f, "{a} and {b} exclude each other (both are in the group {group}); choose one")
            }
            Error::Cycle(id) => write!(f, "the profile {id} inherits from itself"),
        }
    }
}

impl std::error::Error for Error {}

/// What the user chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub flavor: String,
    pub mix_ins: Vec<String>,
}

/// What a [`Selection`] amounts to: everything in application order, duplicates removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Effective {
    /// Full ids in the order they apply (parents before children).
    pub layers: Vec<String>,
    pub atoms: Vec<String>,
    pub testing: Vec<String>,
    pub use_lines: Vec<String>,
    pub licenses: Vec<String>,
    pub services: Vec<String>,
    /// `(key, value, append)`: later profiles win per key, `+=` words are collected from all of them. `append`: [`render`] adds the
    /// words to what the file already sets instead of replacing it.
    pub make_conf: Vec<(String, String, bool)>,
    pub guru: bool,
    pub desktop: bool,
    pub rebuild: Vec<String>,
}

#[derive(Debug, Default)]
struct Raw {
    files: BTreeMap<String, String>,
}

/// The profile data: every flavor and mix-in.
#[derive(Debug, Default)]
pub struct Repo {
    layers: BTreeMap<String, Raw>,
    flavor_order: Vec<String>,
    mix_in_order: Vec<String>,
}

/// Non-empty lines without comments.
fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

impl Repo {
    /// The data compiled into this crate.
    pub fn bundled() -> &'static Repo {
        static REPO: OnceLock<Repo> = OnceLock::new();
        REPO.get_or_init(|| Repo::from_files(BUNDLE.iter().map(|(p, c)| (p.to_string(), c.to_string()))))
    }

    /// From `(path relative to profiles/, contents)` pairs.
    pub fn from_files(files: impl IntoIterator<Item = (String, String)>) -> Repo {
        let mut repo = Repo::default();
        for (path, content) in files {
            let parts: Vec<&str> = path.split('/').collect();
            match parts.as_slice() {
                ["flavor", "index"] => repo.flavor_order = lines(&content),
                ["mix-in", "index"] => repo.mix_in_order = lines(&content),
                [kind @ ("flavor" | "mix-in"), name, file] => {
                    repo.layers
                        .entry(format!("{kind}/{name}"))
                        .or_default()
                        .files
                        .insert(file.to_string(), content);
                }
                _ => {}
            }
        }
        repo
    }

    /// From a directory laid out like `profiles/` (for tests and for working on the data).
    pub fn from_dir(dir: &Path) -> std::io::Result<Repo> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, String)>) -> std::io::Result<()> {
            for e in std::fs::read_dir(dir)? {
                let p = e?.path();
                if p.is_dir() {
                    walk(&p, base, out)?;
                } else if let Ok(text) = std::fs::read_to_string(&p) {
                    out.push((p.strip_prefix(base).unwrap().to_string_lossy().to_string(), text));
                }
            }
            Ok(())
        }
        let mut files = Vec::new();
        walk(dir, dir, &mut files)?;
        Ok(Repo::from_files(files))
    }

    fn raw(&self, full: &str) -> Option<&Raw> {
        self.layers.get(full)
    }

    fn file(&self, full: &str, name: &str) -> Vec<String> {
        self.raw(full).and_then(|r| r.files.get(name)).map(|t| lines(t)).unwrap_or_default()
    }

    fn full_id(kind: Kind, id: &str) -> String {
        if id.contains('/') {
            id.to_string()
        } else {
            format!("{}/{id}", kind.dir())
        }
    }

    /// Flavors in the order of `flavor/index` (the hidden ones too).
    pub fn flavors(&self) -> Vec<Info> {
        self.flavor_order.iter().filter_map(|id| self.info(Kind::Flavor, id)).collect()
    }

    /// Every mix-in in the order of `mix-in/index`.
    pub fn mix_ins(&self) -> Vec<Info> {
        self.mix_in_order.iter().filter_map(|id| self.info(Kind::MixIn, id)).collect()
    }

    /// The mix-ins that are software to tick (not the graphics cards).
    pub fn software(&self) -> Vec<Info> {
        self.mix_ins().into_iter().filter(|m| !m.graphics).collect()
    }

    /// The graphics-card mix-ins.
    pub fn graphics(&self) -> Vec<Info> {
        self.mix_ins().into_iter().filter(|m| m.graphics).collect()
    }

    pub fn flavor_names(&self) -> Vec<String> {
        self.flavor_order.clone()
    }

    pub fn mix_in_names(&self) -> Vec<String> {
        self.mix_in_order.clone()
    }

    pub fn info(&self, kind: Kind, id: &str) -> Option<Info> {
        let full = Self::full_id(kind, id);
        let raw = self.raw(&full)?;
        let short = full.split_once('/').map(|x| x.1).unwrap_or(&full).to_string();
        let desc = raw.files.get("desc").map(|t| lines(t)).unwrap_or_default();
        let needs = self.file(&full, "needs");
        let mut atoms = Vec::new();
        let mut testing = Vec::new();
        for a in self.file(&full, "atoms") {
            match a.strip_prefix('~') {
                Some(plain) => {
                    atoms.push(plain.to_string());
                    testing.push(plain.to_string());
                }
                None => atoms.push(a),
            }
        }
        let make_conf = self
            .file(&full, "make-conf")
            .iter()
            .filter_map(|l| {
                let (k, v) = l.split_once('=')?;
                let (k, append) = match k.trim().strip_suffix('+') {
                    Some(k) => (k.trim(), true),
                    None => (k.trim(), false),
                };
                Some((k.to_string(), v.trim().trim_matches('"').to_string(), append))
            })
            .collect();
        let desktop = raw.files.get("desktop").map(|t| matches!(t.trim(), "yes" | "true" | "1"));
        let parents = self.file(&full, "parent");
        let defaults = if kind == Kind::Flavor { self.inherited_defaults(&full, &mut Vec::new()) } else { Vec::new() };
        Some(Info {
            id: short,
            kind,
            name: desc.first().cloned().unwrap_or_else(|| full.clone()),
            description: desc.iter().skip(1).cloned().collect::<Vec<_>>().join(" "),
            group: self.file(&full, "group").into_iter().next(),
            guru: needs.iter().any(|n| n == "guru"),
            needs_desktop: needs.iter().any(|n| n == "desktop"),
            graphics: raw.files.contains_key("graphics"),
            desktop,
            hidden: raw.files.contains_key("hidden"),
            parents,
            defaults,
            atoms,
            testing,
            use_lines: self.file(&full, "use"),
            licenses: self.file(&full, "licenses"),
            services: self.file(&full, "services"),
            make_conf,
            rebuild: self.file(&full, "rebuild"),
        })
    }

    fn inherited_defaults(&self, full: &str, seen: &mut Vec<String>) -> Vec<String> {
        if seen.iter().any(|s| s == full) {
            return Vec::new();
        }
        seen.push(full.to_string());
        let mut set = BTreeSet::new();
        for p in self.file(full, "parent") {
            set.extend(self.inherited_defaults(&p, seen));
        }
        set.extend(self.file(full, "defaults"));
        // canonical order: the order the mix-ins are offered in
        let mut ordered: Vec<String> = self.mix_in_order.iter().filter(|m| set.contains(*m)).cloned().collect();
        ordered.extend(set.into_iter().filter(|m| !self.mix_in_order.contains(m)));
        ordered
    }

    /// The layers a profile brings, parents first; each layer once.
    fn expand(&self, kind: Kind, id: &str, out: &mut Vec<String>, stack: &mut Vec<String>) -> Result<(), Error> {
        let full = Self::full_id(kind, id);
        if self.raw(&full).is_none() {
            let known = if kind == Kind::Flavor { self.flavor_order.clone() } else { self.mix_in_order.clone() };
            return Err(Error::Unknown {
                kind: if kind == Kind::Flavor { "flavor" } else { "mix-in" },
                id: id.to_string(),
                known,
            });
        }
        if stack.contains(&full) {
            return Err(Error::Cycle(full));
        }
        if out.contains(&full) {
            return Ok(());
        }
        stack.push(full.clone());
        for parent in self.file(&full, "parent") {
            let (pk, pid) = match parent.split_once('/') {
                Some(("flavor", n)) => (Kind::Flavor, n.to_string()),
                Some(("mix-in", n)) => (Kind::MixIn, n.to_string()),
                _ => (kind, parent.clone()),
            };
            self.expand(pk, &pid, out, stack)?;
        }
        stack.pop();
        out.push(full);
        Ok(())
    }

    /// Works out what a choice amounts to. Unknown names and two mix-ins of one group are errors.
    pub fn resolve(&self, sel: &Selection) -> Result<Effective, Error> {
        let mut layers = Vec::new();
        self.expand(Kind::Flavor, &sel.flavor, &mut layers, &mut Vec::new())?;
        let mut chosen: Vec<Info> = Vec::new();
        for m in &sel.mix_ins {
            self.expand(Kind::MixIn, m, &mut layers, &mut Vec::new())?;
            let info = self.info(Kind::MixIn, m).expect("expanded above");
            if !chosen.iter().any(|c| c.id == info.id) {
                chosen.push(info);
            }
        }
        for (i, a) in chosen.iter().enumerate() {
            for b in &chosen[i + 1..] {
                if let (Some(ga), Some(gb)) = (&a.group, &b.group) {
                    if ga == gb {
                        return Err(Error::Conflict { group: ga.clone(), a: a.id.clone(), b: b.id.clone() });
                    }
                }
            }
        }
        let mut eff = Effective { desktop: true, ..Default::default() };
        let push_unique = |v: &mut Vec<String>, x: &str| {
            if !v.iter().any(|y| y == x) {
                v.push(x.to_string());
            }
        };
        for full in &layers {
            let kind = if full.starts_with("flavor/") { Kind::Flavor } else { Kind::MixIn };
            let info = self.info(kind, full).expect("layer exists");
            for a in &info.atoms {
                push_unique(&mut eff.atoms, a);
            }
            for a in &info.testing {
                push_unique(&mut eff.testing, a);
            }
            for l in &info.use_lines {
                push_unique(&mut eff.use_lines, l);
            }
            for l in &info.licenses {
                push_unique(&mut eff.licenses, l);
            }
            for s in &info.services {
                push_unique(&mut eff.services, s);
            }
            for a in &info.rebuild {
                push_unique(&mut eff.rebuild, a);
            }
            for (k, v, append) in &info.make_conf {
                match eff.make_conf.iter_mut().find(|(ek, _, _)| ek == k) {
                    Some(slot) if *append => {
                        for w in v.split_whitespace() {
                            if !slot.1.split_whitespace().any(|x| x == w) {
                                slot.1 = format!("{} {w}", slot.1).trim().to_string();
                            }
                        }
                        slot.2 = true;
                    }
                    Some(slot) => {
                        slot.1 = v.clone();
                        slot.2 = false;
                    }
                    None => eff.make_conf.push((k.clone(), v.split_whitespace().collect::<Vec<_>>().join(" "), *append)),
                }
            }
            eff.guru |= info.guru;
            if let Some(d) = info.desktop {
                eff.desktop = d;
            }
        }
        eff.layers = layers;
        Ok(eff)
    }

    /// Everything wrong with the data: unknown parents and defaults, entries missing from an index, malformed packages.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (kind, order) in [(Kind::Flavor, &self.flavor_order), (Kind::MixIn, &self.mix_in_order)] {
            for id in order {
                if self.raw(&format!("{}/{id}", kind.dir())).is_none() {
                    out.push(format!("{}/index lists {id}, which has no directory", kind.dir()));
                }
            }
        }
        for full in self.layers.keys() {
            let short = full.split_once('/').map(|x| x.1).unwrap_or(full);
            let (kind, order) = if full.starts_with("flavor/") {
                (Kind::Flavor, &self.flavor_order)
            } else {
                (Kind::MixIn, &self.mix_in_order)
            };
            if !order.iter().any(|o| o == short) {
                out.push(format!("{full} is missing from {}/index", kind.dir()));
            }
            if !self.layers[full].files.contains_key("desc") {
                out.push(format!("{full} has no desc"));
            }
            for p in self.file(full, "parent") {
                if self.raw(&p).is_none() {
                    out.push(format!("{full}: unknown parent {p}"));
                }
            }
            for d in self.file(full, "defaults") {
                if self.raw(&format!("mix-in/{d}")).is_none() {
                    out.push(format!("{full}: default mix-in {d} does not exist"));
                }
            }
            if let Some(info) = self.info(kind, short) {
                for a in &info.atoms {
                    let ok = a.split_once('/').is_some_and(|(c, n)| !c.is_empty() && !n.is_empty() && !a.contains(' '));
                    if !ok {
                        out.push(format!("{full}: {a:?} is not category/name"));
                    }
                }
            }
        }
        out
    }
}

impl Effective {
    /// The files the binhost build needs so that the packages it builds have the USE flags an install will ask for
    /// (`iso/binhost/`): `(path, contents)`.
    pub fn binhost_spec(&self) -> Vec<(String, String)> {
        let join = |v: &[String]| v.iter().map(|l| format!("{l}\n")).collect::<String>();
        let keywords: String = self.testing.iter().map(|a| format!("{a} ~amd64\n")).collect();
        vec![
            ("world-groups".to_string(), join(&self.atoms)),
            ("package.accept_keywords/gentoo-installer-packages".to_string(), keywords),
            ("package.use/gentoo-installer-packages".to_string(), join(&self.use_lines)),
            ("package.license/gentoo-installer-packages".to_string(), join(&self.licenses)),
        ]
    }
}

/// How [`render`] writes.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    /// Always accept `~amd64` for everything of the GURU overlay (its packages' dependencies are `~amd64` too), not only when a
    /// chosen profile needs GURU.
    pub guru_wildcard: bool,
}

/// `/etc/portage/package.use` is a file on some systems and a directory on others, and Portage takes one or the other. When it is a
/// file, it becomes a directory that keeps the old contents as `00-original`.
fn ensure_config_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_file() {
        let old = std::fs::read(dir)?;
        std::fs::remove_file(dir)?;
        std::fs::create_dir_all(dir)?;
        if !old.iter().all(u8::is_ascii_whitespace) {
            std::fs::write(dir.join("00-original"), old)?;
        }
    } else {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

fn write_managed(path: &Path, header: &str, body: &str) -> std::io::Result<Option<PathBuf>> {
    if body.is_empty() {
        if path.is_file() {
            std::fs::remove_file(path)?;
        }
        return Ok(None);
    }
    if let Some(dir) = path.parent() {
        ensure_config_dir(dir)?;
    }
    std::fs::write(path, format!("{MANAGED}\n{header}\n{body}"))?;
    Ok(Some(path.to_path_buf()))
}

/// Sets `KEY="value"` in a make.conf: replaces the line that sets the key, or appends one. Returns whether the file changed.
pub fn set_make_conf_key(path: &Path, key: &str, value: &str) -> std::io::Result<bool> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let new_line = format!("{key}=\"{value}\"");
    let mut found = false;
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        let is_key = l.trim_start().strip_prefix(key).is_some_and(|r| r.trim_start().starts_with('='));
        if is_key {
            if !found {
                out.push(new_line.clone());
                found = true;
            }
        } else {
            out.push(l.to_string());
        }
    }
    if !found {
        out.push(new_line);
    }
    let mut next = out.join("\n");
    next.push('\n');
    if next == text {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, next)?;
    Ok(true)
}

/// Adds words to a make.conf variable (`VIDEO_CARDS`) without removing the ones it has. Returns whether the file changed.
pub fn add_make_conf_words(path: &Path, key: &str, words: &str) -> std::io::Result<bool> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut have: Vec<String> = Vec::new();
    for l in text.lines() {
        let l = l.trim_start();
        if let Some(rest) = l.strip_prefix(key).map(str::trim_start).and_then(|r| r.strip_prefix('=')) {
            have = rest.trim().trim_matches('"').split_whitespace().map(String::from).collect();
        }
    }
    let mut changed = false;
    for w in words.split_whitespace() {
        if !have.iter().any(|h| h == w) {
            have.push(w.to_string());
            changed = true;
        }
    }
    if !changed && text.lines().any(|l| l.trim_start().starts_with(key)) {
        return Ok(false);
    }
    set_make_conf_key(path, key, &have.join(" "))
}

/// Writes the Portage configuration of a choice under `root` (`/` on a running system, the mounted target in the installer):
/// `package.use`, `package.accept_keywords`, `package.license` (each as `slp`), the set `@slp` with the packages, and the
/// `make.conf` keys. Returns the files written.
pub fn render(root: &Path, sel: &Selection, eff: &Effective, opts: RenderOptions) -> std::io::Result<Vec<PathBuf>> {
    let portage = root.join("etc/portage");
    let header = format!("# flavor: {}; mix-ins: {}", sel.flavor, if sel.mix_ins.is_empty() { "-".to_string() } else { sel.mix_ins.join(" ") });
    let join = |v: &[String]| v.iter().map(|l| format!("{l}\n")).collect::<String>();
    let mut keywords: String = eff.testing.iter().map(|a| format!("{a} ~amd64\n")).collect();
    if (eff.guru || opts.guru_wildcard) && !eff.layers.is_empty() && !eff.atoms.is_empty() {
        keywords.push_str("*/*::guru ~amd64\n");
    }
    let mut written = Vec::new();
    for (rel, body) in [
        ("package.use/slp", join(&eff.use_lines)),
        ("package.accept_keywords/slp", keywords),
        ("package.license/slp", join(&eff.licenses)),
        ("sets/slp", join(&eff.atoms)),
    ] {
        written.extend(write_managed(&portage.join(rel), &header, &body)?);
    }
    for (k, v, append) in &eff.make_conf {
        let path = portage.join("make.conf");
        let changed = if *append { add_make_conf_words(&path, k, v)? } else { set_make_conf_key(&path, k, v)? };
        if changed {
            written.push(path);
        }
    }
    Ok(written)
}

/// What is chosen on a system: kept in `etc/slp/state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State(pub Selection);

impl State {
    fn path(root: &Path) -> PathBuf {
        root.join("etc/slp/state")
    }

    pub fn load(root: &Path) -> std::io::Result<Option<State>> {
        let text = match std::fs::read_to_string(Self::path(root)) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut flavor = None;
        let mut mix_ins = Vec::new();
        for l in lines(&text) {
            if let Some(v) = l.strip_prefix("flavor=") {
                flavor = Some(v.trim().to_string());
            } else if let Some(v) = l.strip_prefix("mix-in=") {
                mix_ins.push(v.trim().to_string());
            }
        }
        Ok(flavor.map(|flavor| State(Selection { flavor, mix_ins })))
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let path = Self::path(root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = String::from("# The Simple Linux profile of this system; change it with `slp`.\n");
        text.push_str(&format!("flavor={}\n", self.0.flavor));
        for m in &self.0.mix_ins {
            text.push_str(&format!("mix-in={m}\n"));
        }
        std::fs::write(path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(files: &[(&str, &str)]) -> Repo {
        Repo::from_files(files.iter().map(|(p, c)| (p.to_string(), c.to_string())))
    }

    fn tiny() -> Repo {
        repo(&[
            ("flavor/index", "base\nbig\n"),
            ("flavor/base/desc", "Base\nThe basics.\n"),
            ("flavor/base/desktop", "no\n"),
            ("flavor/base/defaults", "tools\n"),
            ("flavor/big/desc", "Big\nMore.\n"),
            ("flavor/big/parent", "flavor/base\n"),
            ("flavor/big/desktop", "yes\n"),
            ("flavor/big/defaults", "game\n"),
            ("flavor/big/use", "dev-foo/bar baz\n"),
            ("mix-in/index", "tools\ngame\ngfx-a\ngfx-b\ngfx-c\n"),
            ("mix-in/tools/desc", "Tools\nSome tools.\n"),
            ("mix-in/tools/atoms", "app-misc/a\n# a comment\n\napp-misc/b\n"),
            ("mix-in/game/desc", "Games\nPlay.\n"),
            ("mix-in/game/atoms", "~games-util/c\napp-misc/a\n"),
            ("mix-in/game/needs", "guru\ndesktop\n"),
            ("mix-in/game/licenses", "games-util/c some-licence\n"),
            ("mix-in/game/services", "gamed\n"),
            ("mix-in/gfx-a/desc", "A\nCard A.\n"),
            ("mix-in/gfx-a/group", "gfx\n"),
            ("mix-in/gfx-a/make-conf", "VIDEO_CARDS+=\"a\"\n"),
            ("mix-in/gfx-a/graphics", "\n"),
            ("mix-in/gfx-b/desc", "B\nCard B.\n"),
            ("mix-in/gfx-b/group", "gfx\n"),
            ("mix-in/gfx-b/make-conf", "VIDEO_CARDS+=\"b c\"\n"),
            ("mix-in/gfx-b/graphics", "\n"),
            ("mix-in/gfx-c/desc", "C\nCard C.\n"),
            ("mix-in/gfx-c/graphics", "\n"),
            ("mix-in/gfx-c/make-conf", "VIDEO_CARDS+=\"c a\"\n"),
        ])
    }

    fn sel(flavor: &str, mix: &[&str]) -> Selection {
        Selection { flavor: flavor.into(), mix_ins: mix.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn a_flavor_inherits_its_parents_defaults_in_the_order_of_the_index() {
        let r = tiny();
        let big = r.info(Kind::Flavor, "big").unwrap();
        assert_eq!(big.defaults, vec!["tools", "game"]);
        assert_eq!(r.info(Kind::Flavor, "base").unwrap().defaults, vec!["tools"]);
        assert_eq!(big.parents, vec!["flavor/base"]);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored_and_testing_atoms_are_marked() {
        let r = tiny();
        assert_eq!(r.info(Kind::MixIn, "tools").unwrap().atoms, vec!["app-misc/a", "app-misc/b"]);
        let g = r.info(Kind::MixIn, "game").unwrap();
        assert_eq!(g.atoms, vec!["games-util/c", "app-misc/a"]);
        assert_eq!(g.testing, vec!["games-util/c"]);
        assert!(g.guru && g.needs_desktop);
        assert_eq!(g.name, "Games");
        assert_eq!(g.description, "Play.");
    }

    #[test]
    fn resolving_applies_parents_first_and_drops_duplicates() {
        let r = tiny();
        let e = r.resolve(&sel("big", &["tools", "game"])).unwrap();
        assert_eq!(e.layers, vec!["flavor/base", "flavor/big", "mix-in/tools", "mix-in/game"]);
        assert_eq!(e.atoms, vec!["app-misc/a", "app-misc/b", "games-util/c"]);
        assert_eq!(e.testing, vec!["games-util/c"]);
        assert_eq!(e.use_lines, vec!["dev-foo/bar baz"]);
        assert_eq!(e.licenses, vec!["games-util/c some-licence"]);
        assert_eq!(e.services, vec!["gamed"]);
        assert!(e.guru && e.desktop);
        assert!(!r.resolve(&sel("base", &[])).unwrap().desktop, "the nearest flavor that says so decides");
    }

    #[test]
    fn unknown_names_and_exclusive_mix_ins_are_errors() {
        let r = tiny();
        match r.resolve(&sel("nope", &[])) {
            Err(Error::Unknown { kind: "flavor", id, known }) => assert!(id == "nope" && known == vec!["base", "big"]),
            other => panic!("{other:?}"),
        }
        assert!(matches!(r.resolve(&sel("base", &["gamex"])), Err(Error::Unknown { kind: "mix-in", .. })));
        let e = r.resolve(&sel("base", &["gfx-a", "gfx-b"])).unwrap_err();
        assert!(matches!(e, Error::Conflict { ref group, .. } if group == "gfx"), "{e}");
        assert!(r.resolve(&sel("base", &["gfx-a", "gfx-a"])).is_ok(), "the same one twice is not a conflict");
    }

    #[test]
    fn make_conf_words_are_collected_from_every_profile_that_adds_them() {
        let r = tiny();
        let e = r.resolve(&sel("base", &["gfx-a"])).unwrap();
        assert_eq!(e.make_conf, vec![("VIDEO_CARDS".to_string(), "a".to_string(), true)]);
        let e = r.resolve(&sel("base", &["gfx-c", "gfx-a"])).unwrap();
        assert_eq!(e.make_conf, vec![("VIDEO_CARDS".to_string(), "c a".to_string(), true)], "a word once, in the order met");
        let r = repo(&[
            ("flavor/index", "f\n"), ("flavor/f/desc", "F\n"), ("flavor/f/make-conf", "X=\"one\"\n"),
            ("mix-in/index", "m\n"), ("mix-in/m/desc", "M\n"), ("mix-in/m/make-conf", "X=\"two\"\n"),
        ]);
        assert_eq!(r.resolve(&sel("f", &["m"])).unwrap().make_conf, vec![("X".to_string(), "two".to_string(), false)], "plain: the later one wins");
    }

    #[test]
    fn graphics_cards_are_kept_apart_from_software() {
        let r = tiny();
        assert_eq!(r.software().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["tools", "game"]);
        assert_eq!(r.graphics().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["gfx-a", "gfx-b", "gfx-c"]);
    }

    #[test]
    fn a_profile_that_inherits_from_itself_is_an_error() {
        let r = repo(&[("flavor/index", "x\n"), ("flavor/x/desc", "X\n"), ("flavor/x/parent", "flavor/x\n"), ("mix-in/index", "")]);
        assert!(matches!(r.resolve(&sel("x", &[])), Err(Error::Cycle(_))));
    }

    #[test]
    fn problems_names_every_inconsistency() {
        let r = repo(&[
            ("flavor/index", "a\nghost\n"),
            ("flavor/a/desc", "A\n"),
            ("flavor/a/parent", "flavor/missing\n"),
            ("flavor/a/defaults", "nomixin\n"),
            ("flavor/b/desc", "B\n"),
            ("mix-in/index", "m\n"),
            ("mix-in/m/atoms", "notanatom\n"),
        ]);
        let p = r.problems().join("\n");
        for needle in [
            "flavor/index lists ghost",
            "flavor/b is missing from flavor/index",
            "unknown parent flavor/missing",
            "default mix-in nomixin does not exist",
            "mix-in/m has no desc",
            "\"notanatom\" is not category/name",
        ] {
            assert!(p.contains(needle), "{needle:?} not in:\n{p}");
        }
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("slp-profiles-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn render_writes_the_portage_files_and_the_make_conf_keys() {
        let r = tiny();
        let s = sel("big", &["tools", "game", "gfx-a"]);
        let e = r.resolve(&s).unwrap();
        let root = temp("render");
        std::fs::create_dir_all(root.join("etc/portage")).unwrap();
        std::fs::write(root.join("etc/portage/make.conf"), "CFLAGS=\"-O2\"\nVIDEO_CARDS=\"old\"\nMAKEOPTS=\"-j4\"\n").unwrap();
        let files = render(&root, &s, &e, RenderOptions::default()).unwrap();
        assert!(files.len() >= 5);
        let read = |p: &str| std::fs::read_to_string(root.join("etc/portage").join(p)).unwrap();
        assert!(read("package.use/slp").ends_with("dev-foo/bar baz\n"));
        assert!(read("package.use/slp").starts_with(MANAGED));
        assert!(read("package.accept_keywords/slp").contains("games-util/c ~amd64\n*/*::guru ~amd64\n"));
        assert!(read("package.license/slp").contains("games-util/c some-licence\n"));
        assert_eq!(read("sets/slp").lines().skip(2).collect::<Vec<_>>(), vec!["app-misc/a", "app-misc/b", "games-util/c"]);
        assert_eq!(read("make.conf"), "CFLAGS=\"-O2\"\nVIDEO_CARDS=\"old a\"\nMAKEOPTS=\"-j4\"\n", "the words are added to what the file has");
        // rendering a smaller choice removes what is no longer needed
        let s2 = sel("base", &["tools"]);
        render(&root, &s2, &r.resolve(&s2).unwrap(), RenderOptions::default()).unwrap();
        assert!(!root.join("etc/portage/package.license/slp").exists());
        assert!(!root.join("etc/portage/package.accept_keywords/slp").exists());
        assert_eq!(std::fs::read_to_string(root.join("etc/portage/make.conf")).unwrap().matches("VIDEO_CARDS").count(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_plain_file_where_portage_wants_a_directory_is_turned_into_one_without_losing_it() {
        let r = tiny();
        let s = sel("big", &["tools", "game"]);
        let e = r.resolve(&s).unwrap();
        let root = temp("filedir");
        std::fs::create_dir_all(root.join("etc/portage")).unwrap();
        std::fs::write(root.join("etc/portage/package.use"), "dev-foo/old flag\n").unwrap();
        std::fs::write(root.join("etc/portage/package.accept_keywords"), "\n").unwrap();
        render(&root, &s, &e, RenderOptions::default()).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("etc/portage/package.use/00-original")).unwrap(), "dev-foo/old flag\n");
        assert!(root.join("etc/portage/sets/slp").is_file());
        assert!(!root.join("etc/portage/package.accept_keywords/00-original").exists(), "an empty file is not worth keeping");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_guru_wildcard_can_be_forced() {
        let r = tiny();
        let s = sel("base", &["tools"]);
        let e = r.resolve(&s).unwrap();
        let root = temp("guru");
        render(&root, &s, &e, RenderOptions { guru_wildcard: true }).unwrap();
        assert!(std::fs::read_to_string(root.join("etc/portage/package.accept_keywords/slp")).unwrap().contains("*/*::guru ~amd64"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn set_make_conf_key_replaces_appends_and_reports_changes() {
        let root = temp("mk");
        let f = root.join("make.conf");
        assert!(set_make_conf_key(&f, "A", "1").unwrap());
        assert!(!set_make_conf_key(&f, "A", "1").unwrap(), "no change, no write");
        assert!(set_make_conf_key(&f, "A", "2").unwrap());
        std::fs::write(&f, "A=\"x\"\nAB=\"keep\"\nA = \"y\"\n").unwrap();
        set_make_conf_key(&f, "A", "z").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "A=\"z\"\nAB=\"keep\"\n", "AB is another key; a second A line is dropped");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn add_make_conf_words_keeps_what_is_there_and_does_not_touch_an_unchanged_file() {
        let root = temp("add");
        let f = root.join("make.conf");
        std::fs::write(&f, "VIDEO_CARDS=\"amdgpu intel nouveau radeon radeonsi\"\nOTHER=\"x\"\n").unwrap();
        assert!(!add_make_conf_words(&f, "VIDEO_CARDS", "intel radeonsi").unwrap(), "all there already");
        assert!(add_make_conf_words(&f, "VIDEO_CARDS", "virgl intel").unwrap());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "VIDEO_CARDS=\"amdgpu intel nouveau radeon radeonsi virgl\"\nOTHER=\"x\"\n");
        assert!(add_make_conf_words(&f, "NEW", "a b").unwrap());
        assert!(std::fs::read_to_string(&f).unwrap().ends_with("NEW=\"a b\"\n"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn state_round_trips() {
        let root = temp("state");
        assert_eq!(State::load(&root).unwrap(), None);
        let st = State(sel("desktop", &["tools", "gfx-amd"]));
        st.save(&root).unwrap();
        assert_eq!(State::load(&root).unwrap(), Some(st));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_bundled_data_has_no_problems_and_resolves() {
        let r = Repo::bundled();
        assert_eq!(r.problems(), Vec::<String>::new());
        for f in r.flavors() {
            r.resolve(&Selection { flavor: f.id.clone(), mix_ins: f.defaults.clone() }).unwrap();
        }
        let live = r.info(Kind::Flavor, "live").unwrap();
        assert!(live.hidden && live.use_lines.iter().any(|l| l == "media-libs/mesa -llvm"));
    }
}
