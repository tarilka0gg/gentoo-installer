//! One-off integration test: runs the REAL (non-simulated) install pipeline against a
//! loopback-file-backed disk instead of real hardware — the spec's own recommended
//! `--dry-run`-against-a-loopback-file methodology (§10). Not part of the app; a
//! throwaway example for verifying the pipeline end-to-end against the local test store
//! set up in ~/gentoo-installer-test/.
//!
//! Usage: cargo run --example real_install_test -p installer-cli -- /dev/loopN

use installer_core::account::Account;
use installer_core::make_conf::{OptLevel, PackageMode};
use installer_core::wm::WmChoice;
use installer_core::{hardware, install, partition, store};

#[tokio::main]
async fn main() {
    let disk = std::env::args().nth(1).expect("usage: real_install_test <loop-device>");

    let profile = hardware::Profile::detect().expect("hardware detection failed");
    println!("Detected profile: {}", profile.combo());

    let layout = partition::plan_with_swap(&disk, partition::RootFs::Btrfs, 8);
    println!("Layout: {layout:?}");

    let opts = install::InstallOptions {
        layout,
        target: "/mnt/gentoo-installer-test".into(),
        store: store::StoreConfig {
            binhost_url: "http://127.0.0.1:8123".into(),
            overlay_git_url: "file:///home/tarilka0gg/gentoo-installer-test/test-overlay".into(),
            overlay_name: "localrepo".into(),
        },
        kernel_base_name: "gentoo-diy-kernel".into(),
        keyboard_layout: installer_core::keyboard::detect_current(),
        timezone: installer_core::timezone::detect_current().unwrap_or_else(|| "UTC".into()),
        gpu_override: None,
        packages: Vec::new(),
        hostname: "gentoo".into(),
        locales: vec![installer_core::locale::DEFAULT_LOCALE.to_string()],
        account: Account { username: "tester".into(), password: "testpassword123".into() },
        wm: WmChoice::Niri,
        wm_configs_git_url: "file:///home/tarilka0gg/Documents/projects/gentoo-wm-configs".into(),
        opt_level: OptLevel::O2,
        package_mode: PackageMode::Binary,
        simulate: false,
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = tokio::spawn(install::run(opts, tx));

    while let Some(progress) = rx.recv().await {
        println!(">> {progress:?}");
    }

    match handle.await {
        Ok(Ok(())) => println!("RESULT: install::run completed successfully"),
        Ok(Err(e)) => println!("RESULT: install::run returned an error: {e}"),
        Err(e) => println!("RESULT: task panicked: {e}"),
    }
}
