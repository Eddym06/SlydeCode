//! The executable a user double-clicks.
//!
//! On Windows it is built for the GUI subsystem, so no console window appears.
//! The console executable (`slyde`) stays the one for the command line and MCP.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    slyde_app::launcher::run()
}
