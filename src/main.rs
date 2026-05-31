mod data;
mod security;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;

use data::SqliteProjectProvider;
use ui::App;

fn resolve_db_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Unable to resolve HOME directory".to_string())?;
    Ok(home.join(".local/share/unit-projman/projects.db"))
}

fn main() -> Result<(), String> {
    let provider = Arc::new(SqliteProjectProvider::new(resolve_db_path()?)?);
    let workspace_root = std::env::current_dir()
        .map_err(|e| format!("Unable to resolve current directory: {e}"))?;

    let mut app = App::new(provider, workspace_root);
    let mut terminal = ratatui::init();
    let run_result = app
        .run(&mut terminal)
        .map_err(|e| format!("TUI runtime error: {e}"));
    ratatui::restore();

    run_result
}
