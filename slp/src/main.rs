//! `slp`: choose the flavor and the mix-ins of a Simple Linux system, like Funtoo's `epro`.
//!
//! A system is one flavor (`minimal`, `desktop`, `workstation`) plus any number of mix-ins (`gaming`, `office`, `dev`, ...) and its
//! graphics cards (`gfx-*`). `slp` records the choice in `/etc/slp/state` and writes the Portage configuration that goes with it
//! (`package.use`, `package.accept_keywords`, `package.license` and the set `@slp`); `slp install` then installs what is missing.

mod cli;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = cli::execute(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
