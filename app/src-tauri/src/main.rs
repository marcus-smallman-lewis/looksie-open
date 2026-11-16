// No console window behind the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `looksie --mcp`: a tool for the reviewer's own Claude Code or Codex, over standard input and output. No window.
    if std::env::args().skip(1).any(|a| a == "--mcp") {
        std::process::exit(looksie_app_lib::run_mcp());
    }
    looksie_app_lib::run()
}
