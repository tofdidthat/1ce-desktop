mod flp_metadata;
mod key;
mod naming;
mod production_timer;
mod rename;
mod render;
mod scanner;

use serde::Serialize;
use tauri::Manager;

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
        scan_only: false,
        export_engine_ready: cfg!(windows),
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
        .manage(rename::RenameState::default())
        .manage(render::ExportState::default())
        .manage(render::finalize::DeliveryState::default())
        .setup(|app| {
            let timer = production_timer::ProductionTimerState::load(app.handle())
                .map_err(|error| std::io::Error::other(error))?;
            app.manage(timer);
            production_timer::sync_tray(app.handle())
                .map_err(|error| std::io::Error::other(error))?;
            production_timer::start_checkpoint_worker(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                production_timer::checkpoint_now(window.app_handle());
                if production_timer::should_keep_running(window.app_handle()) {
                    let anchor = window
                        .outer_position()
                        .ok()
                        .zip(window.outer_size().ok())
                        .map(|(position, size)| {
                            tauri::PhysicalPosition::new(
                                (position.x + size.width as i32) as f64,
                                (position.y + size.height as i32) as f64,
                            )
                        })
                        .unwrap_or(tauri::PhysicalPosition::new(0.0, 0.0));
                    api.prevent_close();
                    let _ = window.hide();
                    production_timer::show_after_main_hidden(window.app_handle().clone(), anchor);
                } else {
                    window.app_handle().exit(0);
                }
            }
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_app_status,
            scanner::scan_beat_folders,
            preview_beat_name,
            rename::prepare_smart_rename,
            rename::execute_smart_rename,
            rename::pending_smart_renames,
            rename::recover_smart_rename,
            render::get_export_config,
            render::save_export_config,
            render::preview_export_plan,
            render::jobs::start_export,
            render::jobs::cancel_export,
            render::jobs::export_jobs,
            render::jobs::export_status,
            key::analyze_audio_key,
            render::finalize::prepare_delivery,
            render::finalize::execute_delivery,
            render::finalize::resume_delivery,
            render::finalize::export_recovery_directory,
            production_timer::production_timer_snapshot,
            production_timer::set_production_timer_enabled,
            production_timer::production_timer_start,
            production_timer::production_timer_pause,
            production_timer::production_timer_resume,
            production_timer::production_timer_finish,
            production_timer::production_timer_add_manual,
            production_timer::production_timer_update,
            production_timer::production_timer_delete,
            production_timer::production_timer_hide,
            production_timer::production_timer_open_history,
            production_timer::production_timer_show
        ])
        .run(tauri::generate_context!())
        .expect("failed to run 1CE Desktop");
}
