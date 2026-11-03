mod cleanup;
mod convert;
mod docview;
mod entry;
mod fsinfo;
mod fsops;
mod mount;
mod office;
mod places;
mod props;
mod search;
mod sidecar;
mod system;
mod tags;
mod transfer;
mod watch;

use fsinfo::*;
use places::*;
use search::*;
use system::*;
use watch::*;

pub type CmdResult<T> = Result<T, String>;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            use tauri::Manager;
            sidecar::init(app.path().resource_dir().ok());
            Ok(())
        })
        .manage(SearchState::default())
        .manage(WatchState::default())
        .manage(transfer::OpsState::default())
        .manage(cleanup::CleanupState::default())
        .manage(convert::ConvertState::default())
        .manage(office::OfficeState::default())
        .invoke_handler(tauri::generate_handler![
            list_dir,
            stat,
            props::properties,
            places,
            mount::list_drives,
            mount::mount_drive,
            mount::unmount_drive,
            search,
            cancel_search,
            dir_size,
            cancel_sizes,
            read_text,
            complete_path,
            home_dir,
            open_terminal,
            watch,
            fsops::create_dir,
            fsops::create_file,
            fsops::move_pairs,
            transfer::transfer,
            transfer::cancel_op,
            fsops::trashcan::trash_items,
            fsops::trashcan::restore_trashed,
            fsops::trashcan::reveal_trash,
            fsops::delete_items,
            cleanup::cleanup_scan,
            cleanup::cleanup_cancel,
            cleanup::cleanup_children,
            cleanup::cleanup_delete,
            cleanup::cleanup_empty_trash,
            convert::convert_caps,
            convert::convert_images,
            convert::convert_files,
            convert::convert_targets,
            convert::convert_cancel,
            convert::convert_estimate,
            convert::preview::convert_preview,
            convert::preview::image_bytes,
            docview::office_preview,
            tags::tags_get,
            tags::tags_set,
            tags::tags_all,
            tags::tags_find,
            office::office_status,
            office::office_install,
            office::office_cancel,
            office::office_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Morfa");
}
