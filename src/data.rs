use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

pub type AppResult<T> = Result<T, String>;

#[derive(Debug, Clone)]
pub struct ProjectData {
    pub id: i64,
    pub name: String,
    pub path: PathBuf,
    pub last_accessed: i64,
    pub template_id: Option<i64>,
    pub template_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TemplateDef {
    pub name: String,
    #[serde(default)]
    pub structure: HashMap<String, HashMap<String, String>>,
}

pub trait ProjectProvider {
    fn get_all_projects(&self) -> AppResult<Vec<ProjectData>>;
    fn add_project(&self, path: PathBuf) -> AppResult<()>;
    fn remove_project(&self, path: &PathBuf) -> AppResult<()>;
    fn get_templates(&self) -> AppResult<Vec<TemplateDef>>;
}

#[derive(Debug, Clone)]
pub struct SqliteProjectProvider {
    db_path: PathBuf,
}

impl SqliteProjectProvider {
    pub fn new(db_path: impl Into<PathBuf>) -> AppResult<Self> {
        let db_path = db_path.into();
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Unable to create DB directory: {e}"))?;
        }
        let provider = Self { db_path };
        provider.init_schema()?;
        Ok(provider)
    }

    fn init_schema(&self) -> AppResult<()> {
        let conn = self.connection()?;
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS templates (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE
            );
            CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
                last_accessed INTEGER NOT NULL,
                template_id INTEGER,
                FOREIGN KEY(template_id) REFERENCES templates(id)
            );
            ",
        )
        .map_err(|e| format!("Failed to initialize schema: {e}"))?;
        Ok(())
    }

    fn connection(&self) -> AppResult<Connection> {
        let conn = Connection::open(&self.db_path)
            .map_err(|e| format!("Unable to open SQLite DB {}: {e}", self.db_path.display()))?;
        conn.execute("PRAGMA foreign_keys = ON", [])
            .map_err(|e| format!("Failed to enable FK constraints: {e}"))?;
        Ok(conn)
    }

    fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn parse_project_name(path: &Path) -> AppResult<String> {
        path.file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("Could not derive project name from path {}", path.display()))
    }

    fn read_template_hint(path: &Path) -> Option<String> {
        let hint_path = path.join(".unit-template");
        let raw = fs::read_to_string(hint_path).ok()?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn ensure_template_row(conn: &Connection, template_name: &str) -> AppResult<i64> {
        conn.execute(
            "INSERT OR IGNORE INTO templates (name) VALUES (?1)",
            params![template_name],
        )
        .map_err(|e| format!("Failed to insert template '{template_name}': {e}"))?;

        conn.query_row(
            "SELECT id FROM templates WHERE name = ?1",
            params![template_name],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| format!("Failed to resolve template ID for '{template_name}': {e}"))
    }
}

impl ProjectProvider for SqliteProjectProvider {
    fn get_all_projects(&self) -> AppResult<Vec<ProjectData>> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "
                SELECT p.id, p.name, p.path, p.last_accessed, p.template_id, t.name
                FROM projects p
                LEFT JOIN templates t ON t.id = p.template_id
                ORDER BY p.last_accessed DESC, p.name ASC
                ",
            )
            .map_err(|e| format!("Failed to prepare project query: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(ProjectData {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: PathBuf::from(row.get::<_, String>(2)?),
                    last_accessed: row.get(3)?,
                    template_id: row.get(4)?,
                    template_name: row.get(5)?,
                })
            })
            .map_err(|e| format!("Failed to fetch projects: {e}"))?;

        let mut projects = Vec::new();
        for row in rows {
            projects.push(row.map_err(|e| format!("Row decode error: {e}"))?);
        }
        Ok(projects)
    }

    fn add_project(&self, path: PathBuf) -> AppResult<()> {
        let name = Self::parse_project_name(&path)?;
        let conn = self.connection()?;
        let template_id = match Self::read_template_hint(&path) {
            Some(template_name) => Some(Self::ensure_template_row(&conn, &template_name)?),
            None => None,
        };

        conn.execute(
            "
            INSERT INTO projects (name, path, last_accessed, template_id)
            VALUES (?1, ?2, ?3, ?4)
            ",
            params![name, path.to_string_lossy().to_string(), Self::now_unix(), template_id],
        )
        .map_err(|e| format!("Failed to insert project '{}': {e}", path.display()))?;

        Ok(())
    }

    fn remove_project(&self, path: &PathBuf) -> AppResult<()> {
        let conn = self.connection()?;
        let count = conn
            .execute(
                "DELETE FROM projects WHERE path = ?1",
                params![path.to_string_lossy().to_string()],
            )
            .map_err(|e| format!("Failed to delete project '{}': {e}", path.display()))?;
        if count == 0 {
            return Err(format!("Project not found for path {}", path.display()));
        }
        Ok(())
    }

    fn get_templates(&self) -> AppResult<Vec<TemplateDef>> {
        let home = dirs::home_dir().ok_or_else(|| "Unable to resolve HOME directory".to_string())?;
        let template_dir = home.join(".config/unit-projman/templates");
        if !template_dir.exists() {
            return Ok(Vec::new());
        }

        let mut templates = Vec::new();
        let entries = fs::read_dir(&template_dir)
            .map_err(|e| format!("Unable to read template directory {}: {e}", template_dir.display()))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read template entry: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            let raw = fs::read_to_string(&path)
                .map_err(|e| format!("Unable to read template file {}: {e}", path.display()))?;
            let mut parsed: TemplateDef = serde_json::from_str(&raw)
                .map_err(|e| format!("Invalid JSON in {}: {e}", path.display()))?;

            if parsed.name.trim().is_empty() {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    parsed.name = stem.to_string();
                }
            }
            templates.push(parsed);
        }

        templates.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(templates)
    }
}
