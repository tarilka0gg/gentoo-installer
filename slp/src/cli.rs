//! The commands of `slp`. `execute` takes the arguments and the two output streams, so that the tests can run every command against
//! a temporary root (`--root DIR`) and read what it printed.

use slp_profiles::{render, Effective, Info, Kind, RenderOptions, Repo, Selection, State};
use std::io::Write;
use std::path::{Path, PathBuf};

const USAGE: &str = "slp - the flavor and the mix-ins of this system, like Funtoo's epro

usage: slp [--root DIR] <command>

  show                        the flavor, the mix-ins and the graphics of this system
  list [flavors|mix-ins|graphics]
                              what can be chosen (* = in use)
  info <name>                 what a flavor or a mix-in is and what it brings
  init [flavor]               record a profile on a system that has none (the flavor's defaults)
  flavor <name> [--defaults]  change the flavor (--defaults: also tick its default mix-ins)
  mix-in [+]<name> | -<name>  add or remove mix-ins, several at once
  apply                       write the Portage configuration of the profile
  rebuild                     the packages whose build the profile's USE flags change (the live image's build uses it)
  install                     apply, then install what is missing (emerge --noreplace @slp)

The choice is kept in /etc/slp/state. --root works on another system (a mounted target); `install` only on the running one.";

struct Ctx<'a> {
    root: PathBuf,
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
    repo: &'static Repo,
}

pub fn execute(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let mut root = PathBuf::from("/");
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--root" {
            match it.next() {
                Some(r) => root = PathBuf::from(r),
                None => {
                    let _ = writeln!(err, "slp: --root needs a directory");
                    return 2;
                }
            }
        } else if let Some(r) = a.strip_prefix("--root=") {
            root = PathBuf::from(r);
        } else {
            rest.push(a.clone());
        }
    }
    let mut ctx = Ctx { root, out, err, repo: Repo::bundled() };
    let cmd = rest.first().map(String::as_str).unwrap_or("show");
    let tail = if rest.is_empty() { &rest[..] } else { &rest[1..] };
    let result = match cmd {
        "show" => show(&mut ctx),
        "list" => list(&mut ctx, tail.first().map(String::as_str)),
        "info" => info(&mut ctx, tail),
        "init" => init(&mut ctx, tail),
        "flavor" => flavor(&mut ctx, tail),
        "mix-in" | "mixin" | "mix-ins" => mix_in(&mut ctx, tail),
        "apply" => apply(&mut ctx),
        "rebuild" => rebuild(&mut ctx),
        "install" => install(&mut ctx),
        "help" | "-h" | "--help" => {
            let _ = writeln!(ctx.out, "{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(ctx.err, "slp: {e}");
            1
        }
    }
}

fn load(ctx: &Ctx) -> Result<Selection, String> {
    match State::load(&ctx.root).map_err(|e| e.to_string())? {
        Some(State(sel)) => Ok(sel),
        None => Err("this system has no profile recorded (/etc/slp/state); run `slp init` to give it one".to_string()),
    }
}

fn save_and_apply(ctx: &mut Ctx, sel: &Selection) -> Result<Effective, String> {
    let eff = ctx.repo.resolve(sel).map_err(|e| e.to_string())?;
    State(sel.clone()).save(&ctx.root).map_err(|e| format!("saving the profile: {e}"))?;
    render(&ctx.root, sel, &eff, RenderOptions::default()).map_err(|e| format!("writing the Portage configuration: {e}"))?;
    Ok(eff)
}

fn info_of(ctx: &Ctx, kind: Kind, id: &str) -> Result<Info, String> {
    ctx.repo.info(kind, id).ok_or_else(|| {
        let known = if kind == Kind::Flavor { ctx.repo.flavor_names() } else { ctx.repo.mix_in_names() };
        format!("unknown {} {id:?}; known: {}", if kind == Kind::Flavor { "flavor" } else { "mix-in" }, known.join(", "))
    })
}

fn show(ctx: &mut Ctx) -> Result<(), String> {
    let sel = load(ctx)?;
    let eff = ctx.repo.resolve(&sel).map_err(|e| e.to_string())?;
    let flavor = info_of(ctx, Kind::Flavor, &sel.flavor)?;
    let mut inherited: Vec<String> = Vec::new();
    for l in &eff.layers {
        if let Some(n) = l.strip_prefix("flavor/") {
            if n != sel.flavor {
                inherited.push(n.to_string());
            }
        }
    }
    let (graphics, software): (Vec<&String>, Vec<&String>) =
        sel.mix_ins.iter().partition(|m| ctx.repo.info(Kind::MixIn, m).is_some_and(|i| i.graphics));
    let join = |v: &[&String]| if v.is_empty() { "-".to_string() } else { v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ") };
    let _ = writeln!(ctx.out, "flavor:    {} ({})", sel.flavor, flavor.name);
    if !inherited.is_empty() {
        let _ = writeln!(ctx.out, "inherits:  {}", inherited.join(", "));
    }
    let _ = writeln!(ctx.out, "desktop:   {}", if eff.desktop { "yes" } else { "no" });
    let _ = writeln!(ctx.out, "mix-ins:   {}", join(&software));
    let _ = writeln!(ctx.out, "graphics:  {}", join(&graphics));
    let _ = writeln!(ctx.out, "packages:  {} (the set @slp)", eff.atoms.len());
    Ok(())
}

fn list(ctx: &mut Ctx, what: Option<&str>) -> Result<(), String> {
    let sel = State::load(&ctx.root).map_err(|e| e.to_string())?.map(|s| s.0);
    let on_flavor = |id: &str| sel.as_ref().is_some_and(|s| s.flavor == id);
    let on_mix = |id: &str| sel.as_ref().is_some_and(|s| s.mix_ins.iter().any(|m| m == id));
    let want = |k: &str| what.is_none() || what == Some(k);
    if !matches!(what, None | Some("flavors") | Some("mix-ins") | Some("graphics")) {
        return Err(format!("list what? flavors, mix-ins or graphics (not {:?})", what.unwrap_or("")));
    }
    if want("flavors") {
        let _ = writeln!(ctx.out, "Flavors:");
        for f in ctx.repo.flavors().into_iter().filter(|f| !f.hidden) {
            let _ = writeln!(ctx.out, " {} {:<14} {} - {}", if on_flavor(&f.id) { '*' } else { ' ' }, f.id, f.name, f.description);
        }
    }
    if want("mix-ins") {
        let _ = writeln!(ctx.out, "Mix-ins:");
        for m in ctx.repo.software() {
            let _ = writeln!(ctx.out, " {} {:<14} {} - {}", if on_mix(&m.id) { '*' } else { ' ' }, m.id, m.name, m.description);
        }
    }
    if want("graphics") {
        let _ = writeln!(ctx.out, "Graphics:");
        for m in ctx.repo.graphics() {
            let _ = writeln!(ctx.out, " {} {:<18} {} - {}", if on_mix(&m.id) { '*' } else { ' ' }, m.id, m.name, m.description);
        }
    }
    Ok(())
}

fn info(ctx: &mut Ctx, args: &[String]) -> Result<(), String> {
    let name = args.first().ok_or("info what? a flavor or a mix-in name")?;
    let (kind, i) = match ctx.repo.info(Kind::Flavor, name) {
        Some(i) => (Kind::Flavor, i),
        None => (Kind::MixIn, info_of(ctx, Kind::MixIn, name)?),
    };
    let _ = writeln!(ctx.out, "{} ({}): {}", i.id, kind.dir(), i.name);
    let _ = writeln!(ctx.out, "  {}", i.description);
    if !i.parents.is_empty() {
        let _ = writeln!(ctx.out, "  inherits:   {}", i.parents.join(", "));
    }
    if kind == Kind::Flavor && !i.defaults.is_empty() {
        let _ = writeln!(ctx.out, "  ticks:      {}", i.defaults.join(" "));
    }
    if let Some(g) = &i.group {
        let _ = writeln!(ctx.out, "  group:      {g} (only one of the group)");
    }
    let section = |out: &mut dyn Write, label: &str, items: &[String]| {
        if !items.is_empty() {
            let _ = writeln!(out, "  {label:<11} {}", items.join(", "));
        }
    };
    let atoms: Vec<String> = i.atoms.iter().map(|a| if i.testing.contains(a) { format!("{a} (~amd64)") } else { a.clone() }).collect();
    section(ctx.out, "packages:", &atoms);
    section(ctx.out, "use:", &i.use_lines);
    section(ctx.out, "licences:", &i.licenses);
    section(ctx.out, "services:", &i.services);
    let mk: Vec<String> = i.make_conf.iter().map(|(k, v, a)| format!("{k}{}=\"{v}\"", if *a { "+" } else { "" })).collect();
    section(ctx.out, "make.conf:", &mk);
    Ok(())
}

/// The mix-ins a flavor ticks that exist, plus the graphics ones already chosen.
fn defaults_of(ctx: &Ctx, flavor: &str, keep: &[String]) -> Vec<String> {
    let mut mix: Vec<String> = ctx.repo.info(Kind::Flavor, flavor).map(|f| f.defaults).unwrap_or_default();
    for m in keep {
        if ctx.repo.info(Kind::MixIn, m).is_some_and(|i| i.graphics) && !mix.contains(m) {
            mix.push(m.clone());
        }
    }
    mix
}

fn init(ctx: &mut Ctx, args: &[String]) -> Result<(), String> {
    if State::load(&ctx.root).map_err(|e| e.to_string())?.is_some() {
        return Err("this system already has a profile; `slp show` it, `slp flavor` or `slp mix-in` changes it".to_string());
    }
    let flavor = args.first().cloned().unwrap_or_else(|| "desktop".to_string());
    info_of(ctx, Kind::Flavor, &flavor)?;
    let sel = Selection { mix_ins: defaults_of(ctx, &flavor, &[]), flavor };
    save_and_apply(ctx, &sel)?;
    let _ = writeln!(ctx.out, "Profile recorded: flavor {}, mix-ins {}.", sel.flavor, sel.mix_ins.join(" "));
    let _ = writeln!(ctx.out, "Nothing was installed. `slp install` installs what is missing.");
    Ok(())
}

fn flavor(ctx: &mut Ctx, args: &[String]) -> Result<(), String> {
    let name = args.iter().find(|a| !a.starts_with("--")).ok_or("flavor which? see `slp list flavors`")?;
    info_of(ctx, Kind::Flavor, name)?;
    let mut sel = load(ctx)?;
    sel.flavor = name.clone();
    if args.iter().any(|a| a == "--defaults") {
        sel.mix_ins = defaults_of(ctx, name, &sel.mix_ins);
    }
    let eff = save_and_apply(ctx, &sel)?;
    let _ = writeln!(ctx.out, "Flavor: {} ({} packages in @slp). `slp install` installs what is missing.", sel.flavor, eff.atoms.len());
    Ok(())
}

fn mix_in(ctx: &mut Ctx, args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("mix-in which? `slp mix-in +gaming -office`; see `slp list mix-ins`".to_string());
    }
    let mut sel = load(ctx)?;
    let (mut add, mut remove): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    for a in args {
        match a.strip_prefix('-') {
            Some(name) => remove.push(name.to_string()),
            None => add.push(a.strip_prefix('+').unwrap_or(a).to_string()),
        }
    }
    for name in add.iter().chain(&remove) {
        info_of(ctx, Kind::MixIn, name)?;
    }
    // removing first lets one command swap `-gfx-nouveau +gfx-nvidia`
    sel.mix_ins.retain(|m| !remove.contains(m));
    for name in add {
        if !sel.mix_ins.contains(&name) {
            sel.mix_ins.push(name);
        }
    }
    let eff = save_and_apply(ctx, &sel)?;
    let _ = writeln!(ctx.out, "Mix-ins: {} ({} packages in @slp). `slp install` installs what is missing.", sel.mix_ins.join(" "), eff.atoms.len());
    Ok(())
}

fn apply(ctx: &mut Ctx) -> Result<(), String> {
    let sel = load(ctx)?;
    let eff = save_and_apply(ctx, &sel)?;
    let _ = writeln!(ctx.out, "Written: package.use, package.accept_keywords, package.license and the set @slp ({} packages).", eff.atoms.len());
    Ok(())
}

fn rebuild(ctx: &mut Ctx) -> Result<(), String> {
    let sel = load(ctx)?;
    let eff = ctx.repo.resolve(&sel).map_err(|e| e.to_string())?;
    for atom in &eff.rebuild {
        let _ = writeln!(ctx.out, "{atom}");
    }
    Ok(())
}

fn install(ctx: &mut Ctx) -> Result<(), String> {
    if ctx.root != Path::new("/") {
        return Err("install works on the running system only; `slp --root DIR apply` writes the configuration of another one".to_string());
    }
    let sel = load(ctx)?;
    let eff = save_and_apply(ctx, &sel)?;
    let _ = writeln!(ctx.out, "Installing @slp ({} packages)...", eff.atoms.len());
    let status = std::process::Command::new("emerge")
        .args(["--noreplace", "--autounmask-write", "--autounmask-continue", "@slp"])
        .status()
        .map_err(|e| format!("could not run emerge: {e}"))?;
    if !status.success() {
        return Err("emerge did not finish; `slp install` can be run again".to_string());
    }
    for service in &eff.services {
        let _ = std::process::Command::new("rc-update").args(["add", service, "default"]).status();
    }
    let _ = writeln!(ctx.out, "Done.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("slp-cli-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(d.join("etc/portage")).unwrap();
        d
    }

    fn run(root: &Path, args: &[&str]) -> (i32, String, String) {
        let mut a: Vec<String> = vec!["--root".into(), root.to_string_lossy().to_string()];
        a.extend(args.iter().map(|s| s.to_string()));
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = execute(&a, &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn a_system_without_a_profile_is_told_how_to_get_one() {
        let root = temp("none");
        let (code, _, err) = run(&root, &["show"]);
        assert_eq!(code, 1);
        assert!(err.contains("slp init"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn init_records_the_flavor_with_its_defaults_and_writes_the_configuration() {
        let root = temp("init");
        let (code, out, err) = run(&root, &["init", "desktop"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("terminal") && out.contains("browser"), "{out}");
        let (_, shown, _) = run(&root, &["show"]);
        assert!(shown.contains("flavor:    desktop (Desktop)") && shown.contains("inherits:  minimal"), "{shown}");
        assert!(shown.contains("desktop:   yes"), "{shown}");
        let set = std::fs::read_to_string(root.join("etc/portage/sets/slp")).unwrap();
        assert!(set.contains("x11-terms/ghostty") && set.contains("net-wireless/iwd"), "{set}");
        let (code, _, err) = run(&root, &["init"]);
        assert_eq!(code, 1);
        assert!(err.contains("already has a profile"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn mix_ins_are_added_and_removed_and_the_configuration_follows() {
        let root = temp("mix");
        run(&root, &["init", "minimal"]);
        let (code, out, err) = run(&root, &["mix-in", "+gaming", "office"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("gaming") && out.contains("office"), "{out}");
        let set = std::fs::read_to_string(root.join("etc/portage/sets/slp")).unwrap();
        assert!(set.contains("games-util/gamemode") && set.contains("app-office/libreoffice-bin"));
        assert!(std::fs::read_to_string(root.join("etc/portage/package.accept_keywords/slp")).unwrap().contains("games-util/gamemode ~amd64"));
        let (code, _, _) = run(&root, &["mix-in", "-gaming", "-office"]);
        assert_eq!(code, 0);
        let set = std::fs::read_to_string(root.join("etc/portage/sets/slp")).unwrap();
        assert!(!set.contains("gamemode") && !set.contains("libreoffice"));
        let (code, _, err) = run(&root, &["mix-in", "+nonsense"]);
        assert_eq!(code, 1);
        assert!(err.contains("unknown mix-in") && err.contains("gaming"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_alternatives_of_one_graphics_vendor_exclude_each_other_but_can_be_swapped_in_one_command() {
        let root = temp("gfx");
        run(&root, &["init", "desktop"]);
        run(&root, &["mix-in", "+gfx-nouveau", "+gfx-intel"]);
        let (code, _, err) = run(&root, &["mix-in", "+gfx-nvidia"]);
        assert_eq!(code, 1);
        assert!(err.contains("exclude each other"), "{err}");
        let (code, _, err) = run(&root, &["mix-in", "-gfx-nouveau", "+gfx-nvidia"]);
        assert_eq!(code, 0, "{err}");
        let (_, shown, _) = run(&root, &["show"]);
        assert!(shown.contains("graphics:  gfx-intel gfx-nvidia"), "{shown}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn changing_the_flavor_keeps_the_mix_ins_unless_asked() {
        let root = temp("flavor");
        run(&root, &["init", "desktop"]);
        run(&root, &["mix-in", "+gaming"]);
        let (code, _, err) = run(&root, &["flavor", "minimal"]);
        assert_eq!(code, 0, "{err}");
        let (_, shown, _) = run(&root, &["show"]);
        assert!(shown.contains("flavor:    minimal") && shown.contains("desktop:   no") && shown.contains("gaming"), "{shown}");
        run(&root, &["flavor", "workstation", "--defaults"]);
        let (_, shown, _) = run(&root, &["show"]);
        assert!(shown.contains("flavor:    workstation") && shown.contains("dev") && !shown.contains("gaming"), "{shown}");
        let (code, _, err) = run(&root, &["flavor", "nope"]);
        assert_eq!(code, 1);
        assert!(err.contains("unknown flavor"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_marks_what_is_in_use_and_hides_the_live_flavor() {
        let root = temp("list");
        run(&root, &["init", "desktop"]);
        let (code, out, _) = run(&root, &["list"]);
        assert_eq!(code, 0);
        assert!(out.contains(" * desktop") && out.contains("   minimal"), "{out}");
        assert!(out.contains(" * terminal") && out.contains("   gaming"), "{out}");
        assert!(out.contains("gfx-amd"), "{out}");
        let (code, out, _) = run(&root, &["list", "flavors"]);
        assert!(code == 0 && !out.contains("Mix-ins") && !out.contains("live"), "the live flavor is not offered: {out}");
        assert_eq!(run(&root, &["list", "nonsense"]).0, 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn info_shows_what_a_mix_in_brings() {
        let root = temp("info");
        let (code, out, err) = run(&root, &["info", "gfx-nvidia"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("x11-drivers/nvidia-drivers (~amd64)") && out.contains("VIDEO_CARDS+=\"nvidia\"") && out.contains("group:      nvidia"), "{out}");
        let (_, out, _) = run(&root, &["info", "workstation"]);
        assert!(out.contains("inherits:   flavor/desktop") && out.contains("ticks:"), "{out}");
        assert_eq!(run(&root, &["info", "nope"]).0, 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn install_refuses_another_root_and_apply_works_on_it() {
        let root = temp("inst");
        run(&root, &["init", "minimal"]);
        let (code, _, err) = run(&root, &["install"]);
        assert_eq!(code, 1);
        assert!(err.contains("running system"), "{err}");
        let (code, out, _) = run(&root, &["apply"]);
        assert_eq!(code, 0);
        assert!(out.contains("@slp"), "{out}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rebuild_lists_what_the_live_flavor_changes_and_nothing_for_the_others() {
        let root = temp("rebuild");
        run(&root, &["init", "live"]);
        let (code, out, err) = run(&root, &["rebuild"]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(out.lines().collect::<Vec<_>>(), ["media-libs/mesa", "dev-vcs/git", "net-analyzer/tcpdump"]);
        let pkg_use = std::fs::read_to_string(root.join("etc/portage/package.use/slp")).unwrap();
        assert!(pkg_use.contains("media-libs/mesa -llvm") && pkg_use.contains("dev-vcs/git -perl -python -gpg -nls"), "{pkg_use}");
        run(&root, &["flavor", "desktop"]);
        assert_eq!(run(&root, &["rebuild"]).1, "");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unknown_commands_and_a_missing_root_are_errors() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(execute(&["frobnicate".into()], &mut out, &mut err), 1);
        assert!(String::from_utf8(err).unwrap().contains("usage: slp"));
        assert_eq!(execute(&["--root".into()], &mut Vec::new(), &mut Vec::new()), 2);
        let mut out = Vec::new();
        assert_eq!(execute(&["help".into()], &mut out, &mut Vec::new()), 0);
        assert!(String::from_utf8(out).unwrap().contains("slp [--root DIR]"));
    }
}
