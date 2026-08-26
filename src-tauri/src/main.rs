#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod codex_skill;
mod desktop;
mod local_app;
mod macos_installation;
mod managed_tool;
mod managed_tool_dependency;
mod window_layout;

fn main() {
    desktop::run();
}
