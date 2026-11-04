// Hides the extra console window that would otherwise open in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    claws_lib::run()
}
