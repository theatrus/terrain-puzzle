use std::sync::OnceLock;

use tauri::Manager;

static ENGINE_STARTED: OnceLock<()> = OnceLock::new();

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            ENGINE_STARTED.get_or_init(|| {
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        terrain_api::run_with(data_dir, "127.0.0.1:38787".into()).await
                    {
                        eprintln!("terrain engine stopped: {error:#}");
                        app_handle.exit(1);
                    }
                });
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running TopoSaic");
}
