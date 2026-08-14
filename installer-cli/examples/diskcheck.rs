#[tokio::main]
async fn main() {
    match installer_core::disk::list(&installer_core::command::RealCommandRunner).await {
        Ok(disks) => for d in disks { println!("{:?}", d); },
        Err(e) => println!("error: {e}"),
    }
}
