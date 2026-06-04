use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::contract::{HealthState, IndexRunSummary};
use crate::storage::{FileRecord, Store};

#[derive(Debug)]
pub enum IndexError {
    Io(io::Error),
    Sql(rusqlite::Error),
    InvalidRoot(PathBuf),
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Sql(error) => write!(formatter, "{error}"),
            Self::InvalidRoot(path) => write!(
                formatter,
                "repository root does not exist: {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<io::Error> for IndexError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for IndexError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

pub type IndexResult<T> = Result<T, IndexError>;

pub fn index_repository(store: &Store, root: impl AsRef<Path>) -> IndexResult<IndexRunSummary> {
    let root = root.as_ref();
    if !root.exists() {
        return Err(IndexError::InvalidRoot(root.to_path_buf()));
    }

    let started_at_unix_ms = now_unix_ms();
    let records = scan_repository(root)?;
    let scanned_paths = records
        .iter()
        .map(|record| record.path.clone())
        .collect::<HashSet<_>>();
    let mut summary = IndexRunSummary {
        started_at_unix_ms,
        finished_at_unix_ms: started_at_unix_ms,
        scanned_files: records.len() as u64,
        unchanged_files: 0,
        added_files: 0,
        modified_files: 0,
        deleted_files: 0,
        embedded_chunks: 0,
        skipped_embeddings: 0,
        health_state: HealthState::Healthy,
    };

    for record in records {
        match store.file_hash(&record.path)? {
            Some(existing_hash) if existing_hash == record.hash => {
                store.update_file_mtime(&record.path, record.mtime_ms)?;
                summary.unchanged_files += 1;
                summary.skipped_embeddings += 1;
            }
            Some(_) => {
                store.upsert_file(&record)?;
                summary.modified_files += 1;
            }
            None => {
                store.upsert_file(&record)?;
                summary.added_files += 1;
            }
        }
    }

    let indexed_paths = store.all_file_paths()?;
    for indexed_path in indexed_paths {
        if !scanned_paths.contains(&indexed_path) {
            store.delete_file(&indexed_path)?;
            summary.deleted_files += 1;
        }
    }

    summary.finished_at_unix_ms = now_unix_ms();
    store.record_index_run(&summary)?;
    Ok(summary)
}

pub fn scan_repository(root: impl AsRef<Path>) -> IndexResult<Vec<FileRecord>> {
    let root = root.as_ref();
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<FileRecord>) -> IndexResult<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name();
        if should_skip_path(file_name) {
            continue;
        }

        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            collect_files(root, &path, files)?;
        } else if metadata.is_file() {
            let bytes = fs::read(&path)?;
            let hash = hash_bytes(&bytes);
            let (content, language) = match String::from_utf8(bytes) {
                Ok(content) => (content, language_for_path(&path).to_string()),
                Err(_) => (String::new(), "binary".to_string()),
            };
            let relative_path = normalize_relative_path(root, &path)?;
            files.push(FileRecord {
                path: relative_path,
                hash,
                mtime_ms: metadata_mtime_ms(&metadata),
                size_bytes: metadata.len() as i64,
                language,
                content,
            });
        }
    }
    Ok(())
}

fn should_skip_path(file_name: Option<&OsStr>) -> bool {
    matches!(
        file_name.and_then(OsStr::to_str),
        Some(".git" | ".serena" | "node_modules" | "target" | "dist")
    )
}

fn normalize_relative_path(root: &Path, path: &Path) -> IndexResult<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| IndexError::InvalidRoot(root.to_path_buf()))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut hash, "{byte:02x}").expect("writing into String cannot fail");
    }
    hash
}

fn metadata_mtime_ms(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn language_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(OsStr::to_str).unwrap_or_default() {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "json" => "json",
        "md" | "markdown" => "markdown",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        _ => "text",
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn indexes_add_modify_unchanged_and_delete_cycles() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::write(root.join("src/lib.rs"), "pub fn first() {}\n").expect("write lib");
        fs::write(root.join("README.md"), "# fixture\n").expect("write readme");
        let store = Store::open_in_memory().expect("store opens");

        let first = index_repository(&store, root).expect("first index");

        assert_eq!(first.scanned_files, 2);
        assert_eq!(first.added_files, 2);
        assert_eq!(first.modified_files, 0);
        assert_eq!(first.deleted_files, 0);
        assert_eq!(store.file_count().expect("file count"), 2);

        let second = index_repository(&store, root).expect("second index");

        assert_eq!(second.added_files, 0);
        assert_eq!(second.modified_files, 0);
        assert_eq!(second.unchanged_files, 2);
        assert_eq!(second.skipped_embeddings, 2);

        fs::write(root.join("src/lib.rs"), "pub fn second() {}\n").expect("modify lib");
        fs::remove_file(root.join("README.md")).expect("delete readme");
        let third = index_repository(&store, root).expect("third index");

        assert_eq!(third.scanned_files, 1);
        assert_eq!(third.modified_files, 1);
        assert_eq!(third.deleted_files, 1);
        assert_eq!(store.file_content("README.md").expect("read deleted"), None);
        assert_eq!(store.file_count().expect("file count"), 1);
    }

    #[test]
    fn preserves_multibyte_file_content_in_authoritative_store() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::write(
            root.join("src/lib.rs"),
            "pub fn greeting() -> &'static str { \"你好 🚀\" }\n",
        )
        .expect("write lib");
        let store = Store::open_in_memory().expect("store opens");

        index_repository(&store, root).expect("index repository");

        assert_eq!(
            store.file_content("src/lib.rs").expect("content reads"),
            Some("pub fn greeting() -> &'static str { \"你好 🚀\" }\n".to_string())
        );
    }

    #[test]
    fn scan_repository_indexes_binary_files_without_aborting() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::write(root.join("src/lib.rs"), "pub fn text() {}\n").expect("write text");
        fs::write(root.join("src/blob.bin"), [0xff, 0xfe, 0xfd]).expect("write binary");

        let files = scan_repository(root).expect("scan repository");

        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["src/blob.bin", "src/lib.rs"])
        );
        let binary = files
            .iter()
            .find(|file| file.path == "src/blob.bin")
            .expect("binary indexed");
        assert_eq!(binary.language, "binary");
        assert_eq!(binary.content, "");
    }

    #[test]
    fn scan_repository_skips_derived_directories() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join("target/debug")).expect("target dir");
        fs::write(
            root.join("target/debug/generated.rs"),
            "pub fn generated() {}\n",
        )
        .expect("write generated");
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n")
            .expect("write manifest");

        let files = scan_repository(root).expect("scan repository");

        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["Cargo.toml"])
        );
    }
}
