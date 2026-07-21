#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    donutbrowser_lib::run_with_builder(|builder| {
        builder.plugin(tauri_plugin_cross_platform_webdriver::init())
    });
}
