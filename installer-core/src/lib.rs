pub mod bootloader;
pub mod disk;
pub mod hardware;
pub mod kernel;
pub mod network;
pub mod partition;
mod process;
pub mod stage3;
pub mod store;

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
