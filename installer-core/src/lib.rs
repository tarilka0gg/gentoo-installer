pub mod account;
pub mod bootloader;
mod chroot_emerge;
pub mod command;
pub mod config;
pub mod detect;
pub mod disk;
pub mod event;
pub mod fstab;
pub mod gpu_driver;
pub mod hardware;
mod http;
pub mod install;
pub mod journal;
pub mod kernel;
pub mod keyboard;
pub mod locale;
pub mod packages;
pub mod make_conf;
pub mod network;
pub mod partition;
pub mod phase;
pub mod stage3;
pub mod store;
pub mod timezone;
pub mod wm;

pub use error::{Error, Result};

mod error {
    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error("io error: {0}")]
        Io(#[from] std::io::Error),
        #[error("command '{cmd}' failed: {detail}")]
        Command { cmd: String, detail: String },
        #[error("no matching kernel profile found for detected hardware ({0})")]
        NoKernelProfile(String),
        #[error("no matching disk/partitioning target: {0}")]
        Partition(String),
        #[error("network: {0}")]
        Network(String),
        #[error(transparent)]
        Other(#[from] anyhow::Error),
    }

    pub type Result<T> = std::result::Result<T, Error>;
}
