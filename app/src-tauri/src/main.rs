// 发布构建下不带控制台窗口（Windows）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ncm_desktop_lib::run()
}
