use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Result};

use crate::chunking::ChunkRecord;
use crate::contract::{HealthState, IndexRunSummary};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    pub path: String,
    pub hash: String,
    pub mtime_ms: i64,
    pub size_bytes: i64,
    pub content: String,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedFile {
    pub path: String,
    pub hash: String,
    pub size_bytes: i64,
    pub language: String,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.initialize_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.initialize_schema()?;
        Ok(store)
    }

    pub fn initialize_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS files (
               path TEXT PRIMARY KEY,
               hash TEXT NOT NULL,
               mtime_ms INTEGER NOT NULL,
               size_bytes INTEGER NOT NULL,
               content TEXT NOT NULL,
               language TEXT NOT NULL,
               updated_at_unix_ms INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER) * 1000)
             );
             CREATE TABLE IF NOT EXISTS chunks (
               id TEXT PRIMARY KEY,
               file_path TEXT NOT NULL,
               chunk_index INTEGER NOT NULL,
               start_line INTEGER NOT NULL,
               end_line INTEGER NOT NULL,
               start_utf16 INTEGER NOT NULL,
               end_utf16 INTEGER NOT NULL,
               breadcrumb TEXT,
               content TEXT NOT NULL,
               FOREIGN KEY(file_path) REFERENCES files(path) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS chunks_file_path_idx ON chunks(file_path);
             CREATE TABLE IF NOT EXISTS metadata (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS stats (
               key TEXT PRIMARY KEY,
               value INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS index_runs (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               started_at_unix_ms INTEGER NOT NULL,
               finished_at_unix_ms INTEGER NOT NULL,
               scanned_files INTEGER NOT NULL,
               unchanged_files INTEGER NOT NULL,
               added_files INTEGER NOT NULL,
               modified_files INTEGER NOT NULL,
               deleted_files INTEGER NOT NULL,
               embedded_chunks INTEGER NOT NULL,
               skipped_embeddings INTEGER NOT NULL,
               health_state TEXT NOT NULL
             );
             PRAGMA user_version = 1;",
        )?;
        self.set_metadata("schema_version", &SCHEMA_VERSION.to_string())?;
        Ok(())
    }

    pub fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO metadata (key, value)
             VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn metadata(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM metadata WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn increment_stat(&self, key: &str, by: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO stats (key, value)
             VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = value + excluded.value",
            params![key, by],
        )?;
        Ok(())
    }

    pub fn stat(&self, key: &str) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT value FROM stats WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map(|value| value.unwrap_or_default())
    }

    pub fn upsert_file(&self, record: &FileRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO files (path, hash, mtime_ms, size_bytes, content, language)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET
               hash = excluded.hash,
               mtime_ms = excluded.mtime_ms,
               size_bytes = excluded.size_bytes,
               content = excluded.content,
               language = excluded.language,
               updated_at_unix_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000",
            params![
                record.path,
                record.hash,
                record.mtime_ms,
                record.size_bytes,
                record.content,
                record.language
            ],
        )?;
        Ok(())
    }

    pub fn update_file_mtime(&self, path: &str, mtime_ms: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE files SET mtime_ms = ?2 WHERE path = ?1",
            params![path, mtime_ms],
        )?;
        Ok(())
    }

    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(())
    }

    pub fn file_hash(&self, path: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT hash FROM files WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn file_content(&self, path: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT content FROM files WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn replace_file_chunks(&self, file_path: &str, chunks: &[ChunkRecord]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM chunks WHERE file_path = ?1",
            params![file_path],
        )?;
        let mut stmt = self.conn.prepare(
            "INSERT INTO chunks (
               id,
               file_path,
               chunk_index,
               start_line,
               end_line,
               start_utf16,
               end_utf16,
               breadcrumb,
               content
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;

        for chunk in chunks {
            stmt.execute(params![
                chunk.id,
                chunk.file_path,
                chunk.chunk_index,
                chunk.start_line,
                chunk.end_line,
                chunk.start_utf16,
                chunk.end_utf16,
                chunk.breadcrumb,
                chunk.content,
            ])?;
        }

        Ok(())
    }

    pub fn chunks_for_file(&self, file_path: &str) -> Result<Vec<ChunkRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path, chunk_index, start_line, end_line, start_utf16, end_utf16, breadcrumb, content
             FROM chunks
             WHERE file_path = ?1
             ORDER BY chunk_index",
        )?;
        let rows = stmt.query_map(params![file_path], |row| {
            Ok(ChunkRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                chunk_index: row.get(2)?,
                start_line: row.get(3)?,
                end_line: row.get(4)?,
                start_utf16: row.get(5)?,
                end_utf16: row.get(6)?,
                breadcrumb: row.get(7)?,
                content: row.get(8)?,
            })
        })?;
        rows.collect()
    }

    pub fn all_file_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files ORDER BY path")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    pub fn list_files(&self, max_results: Option<u32>) -> Result<Vec<IndexedFile>> {
        let limit = i64::from(max_results.unwrap_or(200));
        let mut stmt = self.conn.prepare(
            "SELECT path, hash, size_bytes, language
             FROM files
             ORDER BY path
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(IndexedFile {
                path: row.get(0)?,
                hash: row.get(1)?,
                size_bytes: row.get(2)?,
                language: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    pub fn file_count(&self) -> Result<u64> {
        self.count_table("files")
    }

    pub fn chunk_count(&self) -> Result<u64> {
        self.count_table("chunks")
    }

    pub fn record_index_run(&self, summary: &IndexRunSummary) -> Result<()> {
        self.conn.execute(
            "INSERT INTO index_runs (
               started_at_unix_ms,
               finished_at_unix_ms,
               scanned_files,
               unchanged_files,
               added_files,
               modified_files,
               deleted_files,
               embedded_chunks,
               skipped_embeddings,
               health_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                summary.started_at_unix_ms,
                summary.finished_at_unix_ms,
                summary.scanned_files,
                summary.unchanged_files,
                summary.added_files,
                summary.modified_files,
                summary.deleted_files,
                summary.embedded_chunks,
                summary.skipped_embeddings,
                health_state_name(&summary.health_state),
            ],
        )?;
        self.increment_stat("index.total_runs", 1)?;
        Ok(())
    }

    fn count_table(&self, table: &str) -> Result<u64> {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        self.conn.query_row(&sql, [], |row| row.get(0))
    }
}

fn health_state_name(state: &HealthState) -> &'static str {
    match state {
        HealthState::Healthy => "healthy",
        HealthState::Degraded => "degraded",
        HealthState::Unavailable => "unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_authoritative_tables_and_schema_metadata() {
        let store = Store::open_in_memory().expect("store opens");

        assert_eq!(
            store.metadata("schema_version").expect("metadata reads"),
            Some(SCHEMA_VERSION.to_string())
        );
        assert_eq!(store.file_count().expect("files count"), 0);
        assert_eq!(store.chunk_count().expect("chunks count"), 0);
    }

    #[test]
    fn upserts_lists_and_deletes_file_content() {
        let store = Store::open_in_memory().expect("store opens");
        let record = FileRecord {
            path: "src/lib.rs".to_string(),
            hash: "hash-1".to_string(),
            mtime_ms: 10,
            size_bytes: 12,
            content: "pub fn lib() {}".to_string(),
            language: "rust".to_string(),
        };

        store.upsert_file(&record).expect("file upserts");

        assert_eq!(
            store.file_hash("src/lib.rs").expect("hash reads"),
            Some("hash-1".to_string())
        );
        assert_eq!(
            store.file_content("src/lib.rs").expect("content reads"),
            Some("pub fn lib() {}".to_string())
        );
        assert_eq!(store.list_files(None).expect("files list").len(), 1);

        store.delete_file("src/lib.rs").expect("file deletes");

        assert_eq!(
            store.file_content("src/lib.rs").expect("content reads"),
            None
        );
    }

    #[test]
    fn replaces_file_chunks_as_a_whole() {
        let store = Store::open_in_memory().expect("store opens");
        let record = FileRecord {
            path: "src/lib.rs".to_string(),
            hash: "hash-1".to_string(),
            mtime_ms: 10,
            size_bytes: 12,
            content: "a🚀文\n".to_string(),
            language: "rust".to_string(),
        };
        store.upsert_file(&record).expect("file upserts");
        let first = vec![ChunkRecord {
            id: "src/lib.rs:hash-1:0".to_string(),
            file_path: "src/lib.rs".to_string(),
            chunk_index: 0,
            start_line: 1,
            end_line: 1,
            start_utf16: 0,
            end_utf16: 5,
            breadcrumb: Some("src/lib.rs".to_string()),
            content: "a🚀文\n".to_string(),
        }];
        let second = vec![ChunkRecord {
            id: "src/lib.rs:hash-2:0".to_string(),
            file_path: "src/lib.rs".to_string(),
            chunk_index: 0,
            start_line: 1,
            end_line: 1,
            start_utf16: 0,
            end_utf16: 4,
            breadcrumb: Some("src/lib.rs".to_string()),
            content: "next".to_string(),
        }];

        store
            .replace_file_chunks("src/lib.rs", &first)
            .expect("first chunks replace");
        store
            .replace_file_chunks("src/lib.rs", &second)
            .expect("second chunks replace");

        let chunks = store.chunks_for_file("src/lib.rs").expect("chunks read");
        assert_eq!(chunks, second);
        assert_eq!(store.chunk_count().expect("chunk count"), 1);
    }
}
