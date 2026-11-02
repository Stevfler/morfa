mod entry;
mod fsinfo;
mod fsops;
mod places;
mod search;
mod system;
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
        .manage(SearchState::default())
        .manage(WatchState::default())
        .manage(transfer::OpsState::default())
        .invoke_handler(tauri::generate_handler![
            list_dir,
            stat,
            places,
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
            fsops::trash_items,
            fsops::restore_trashed,
            fsops::delete_items,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Orbit");
}
