//! The desktop program. Everything it does lives in the library, which the
//! Android app is also built from.

// Release builds have no console window of their own.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    audiovis::run()
}
