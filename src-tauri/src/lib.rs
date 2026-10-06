pub mod build;
pub mod commands;
pub mod contracts;
pub mod domain;
pub mod error;
pub mod paths;
pub mod platform;
pub mod safety;
pub mod services;
pub mod tasks;

use tauri::Manager;

use commands::state::AppStateManager;
use paths::AppPaths;
use safety::flash_auth::FlashSecurityContext;
use tasks::registry::TaskRegistry;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_handle = app.handle().clone();

            let app_data = app.path().app_data_dir()?;
            // Builds and recovery images are large: keep them out of the
            // roaming profile on Windows.
            let local_data = app.path().app_local_data_dir().unwrap_or_else(|_| app_data.clone());
            let app_cache = app.path().app_cache_dir().unwrap_or_else(|_| local_data.join("cache"));
            let paths = AppPaths::new(&local_data, &app_cache);
            // Scratch extraction space never needs to survive a restart.
            let _ = std::fs::remove_dir_all(&paths.work);
            let _ = std::fs::create_dir_all(&paths.work);

            app.manage(AppStateManager::new(app_data, paths.builds.clone()));
            app.manage(paths);
            app.manage(TaskRegistry::new(app_handle));
            app.manage(FlashSecurityContext::new(uuid::Uuid::new_v4().to_string()));

            log::info!("OpCore-OneClick v{APP_VERSION} initialized");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // App
            commands::app::get_app_info,
            commands::app::check_for_updates,
            // Hardware
            commands::hardware::scan_hardware,
            commands::hardware::refresh_profile,
            commands::hardware::get_catalog,
            commands::hardware::export_profile,
            commands::hardware::import_profile,
            // EFI
            commands::efi::check_compatibility,
            commands::efi::plan_build,
            commands::efi::get_bios_settings,
            commands::efi::build_efi,
            commands::efi::validate_efi,
            commands::efi::export_efi,
            // Disk
            commands::disk::list_usb_devices,
            commands::disk::get_disk_info,
            commands::disk::check_privileges,
            commands::disk::flash_prepare_confirmation,
            commands::disk::flash_usb,
            // Firmware
            commands::firmware::probe_firmware,
            // Recovery
            commands::recovery::download_recovery,
            commands::recovery::get_cached_recovery_info,
            commands::recovery::clear_recovery_cache,
            // Diagnostics
            commands::diagnostics::log_get_session_id,
            commands::diagnostics::log_get_tail,
            commands::diagnostics::save_support_log,
            commands::diagnostics::clear_app_cache,
            // State
            commands::state::get_persisted_state,
            commands::state::save_state,
            commands::state::clear_state,
            // Tasks
            commands::task::task_list,
            commands::task::task_cancel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
