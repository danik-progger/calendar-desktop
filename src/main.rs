#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod backend;
mod desktop;
mod editor;
mod icons;
mod model;
mod style;

fn main() {
    desktop::run();
}
