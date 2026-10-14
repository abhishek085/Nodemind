//! Nodemind binary entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    nodemind_lib::run();
}
