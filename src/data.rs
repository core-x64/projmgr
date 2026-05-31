use rusqlite::{Connection, Result};
use std::{fs, path::PathBuf};

pub trait ProjectProvider {
    fn get_all_projects(&self) -> Vec<ProjectData>;
    fn add_project(&mut self, name: &str, path: PathBuf) -> Result<(), String>;
    fn remove_project(&mut self, path: &PathBuf) -> Result<(), String>;
    fn get_templates(&self) -> Vec<String>;
}

pub struct ProjectData {
    pub name: String,
    pub path: PathBuf,
    pub last_accessed: u64,
}

pub struct SqliteProjectProvider {
    conn: Connection,
}

impl SqliteProjectProvider {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
                last_accessed INTEGER
            )",
            [],
        )?;
        Ok(Self { conn })
    }
}

impl ProjectProvider for SqliteProjectProvider {
    fn get_all_projects(&self) -> Vec<ProjectData> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, path, last_accessed FROM projects")
            .unwrap();
        stmt.query_map([], |row| {
            Ok(ProjectData {
                name: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
                last_accessed: row.get(2)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    fn add_project(&mut self, name: &str, path: PathBuf) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO projects (name, path, last_accessed) VALUES (?1, ?2, ?3)",
                [name, &path.to_string_lossy(), &0.to_string()],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn remove_project(&mut self, path: &PathBuf) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM projects WHERE path = ?1",
                [&path.to_string_lossy()],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_templates(&self) -> Vec<String> {
        let path = dirs::config_dir().unwrap().join("projmgr/templates");
        fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}
