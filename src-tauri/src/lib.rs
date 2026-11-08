mod flp_metadata;
mod naming;
mod scanner;

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppStatus {
    version: &'static str,
    scan_only: bool,
    export_engine_ready: bool,
}

#[tauri::command]
fn get_app_status() -> AppStatus {
    AppStatus {
        version: env!("CARGO_PKG_VERSION"),
        scan_only: true,
        export_engine_ready: false,
    }
}

#[tauri::command]
fn preview_beat_name(
    title: String,
    flp_bpm: Option<f64>,
    detected_key: Option<String>,
    template: String,
) -> Result<naming::NamePreview, String> {
    naming::preview_title(&title, flp_bpm, detected_key.as_deref(), &template)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![get_app_status, scanner::scan_beat_folders, preview_beat_name])
        .run(tauri::generate_context!())
        .expect("failed to run 1CE Desktop");
}
