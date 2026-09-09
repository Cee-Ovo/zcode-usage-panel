// Prevents an extra console window on Windows in every build profile.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    zcode_usage_panel_lib::run()
}
