// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = lynx_app_lib::card_worker_entry() {
        std::process::exit(code);
    }
    lynx_app_lib::run()
}
