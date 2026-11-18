//! Read-only release probe. Uses the updater, feed and key from the application configuration.
use sha2::{Digest, Sha256};
use std::error::Error;
use tauri_plugin_updater::UpdaterExt;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let installed = args.next().ok_or("expected installed version")?;
    let target = args.next().ok_or("expected updater target")?;
    let expected = args.next().ok_or("expected installer SHA-256 or 'current'")?;
    let mut context = tauri::generate_context!();
    context.package_info_mut().version = installed.parse()?;
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)?;
    let mut builder = app.updater_builder().target(target);
    if let Some(feed) = args.next() {
        // Staging is limited to a loopback fixture. The actual public check uses embedded settings.
        let url = tauri::Url::parse(&feed)?;
        if url.host_str() != Some("127.0.0.1") {
            return Err("staging feed must be on loopback".into());
        }
        builder = builder.endpoints(vec![url])?;
    }
    let updater = builder.build()?;
    tauri::async_runtime::block_on(async {
        match updater.check().await? {
            None if expected == "current" => println!("Current version {installed}"),
            Some(update) if expected != "current" => {
                let bytes = update.download(|_, _| {}, || {}).await?;
                let actual = hex::encode(Sha256::digest(&bytes));
                if expected != actual {
                    return Err("Downloaded bytes differ from the staged installer".into());
                }
                println!("Verified {}: {} bytes, SHA-256 {}", update.version, bytes.len(), actual);
            }
            _ => return Err("Unexpected update discovery result".into()),
        }
        Ok(())
    })
}
