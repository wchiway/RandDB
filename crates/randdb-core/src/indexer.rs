use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::chunking::{chunk_file, ChunkConfig};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexOptions {
    pub ignored_names: Vec<OsString>,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            ignored_names: [".git", ".serena", "node_modules", "target", "dist"]
                .into_iter()
                .map(OsString::from)
                .collect(),
        }
    }
}

pub fn index_repository(store: &Store, root: impl AsRef<Path>) -> IndexResult<IndexRunSummary> {
    index_repository_with_options(store, root, &IndexOptions::default())
}

pub fn index_repository_with_options(
    store: &Store,
    root: impl AsRef<Path>,
    options: &IndexOptions,
) -> IndexResult<IndexRunSummary> {
    store.begin_index_run()?;
    match index_repository_inner(store, root.as_ref(), options) {
        Ok(summary) => {
            store.complete_index_run(&summary)?;
            Ok(summary)
        }
        Err(error) => {
            store.mark_index_failed(&error.to_string())?;
            Err(error)
        }
    }
}

fn index_repository_inner(
    store: &Store,
    root: &Path,
    options: &IndexOptions,
) -> IndexResult<IndexRunSummary> {
    if !root.exists() {
        return Err(IndexError::InvalidRoot(root.to_path_buf()));
    }

    let started_at_unix_ms = now_unix_ms();
    let records = scan_repository_with_options(root, options)?;
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
                store.replace_file_chunks(
                    &record.path,
                    &chunk_file(&record, &ChunkConfig::default()),
                )?;
                summary.modified_files += 1;
            }
            None => {
                store.upsert_file(&record)?;
                store.replace_file_chunks(
                    &record.path,
                    &chunk_file(&record, &ChunkConfig::default()),
                )?;
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
    Ok(summary)
}

pub fn scan_repository(root: impl AsRef<Path>) -> IndexResult<Vec<FileRecord>> {
    scan_repository_with_options(root, &IndexOptions::default())
}

pub fn scan_repository_with_options(
    root: impl AsRef<Path>,
    options: &IndexOptions,
) -> IndexResult<Vec<FileRecord>> {
    let root = root.as_ref();
    let mut files = Vec::new();
    collect_files(root, root, &mut files, options)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn collect_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<FileRecord>,
    options: &IndexOptions,
) -> IndexResult<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name();
        if should_skip_path(file_name, options) {
            continue;
        }

        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            collect_files(root, &path, files, options)?;
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

fn should_skip_path(file_name: Option<&OsStr>, options: &IndexOptions) -> bool {
    file_name.is_some_and(|name| options.ignored_names.iter().any(|ignored| ignored == name))
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
    fn index_repository_writes_replaces_and_deletes_chunks() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::write(root.join("src/lib.rs"), "a🚀文\n").expect("write lib");
        let store = Store::open_in_memory().expect("store opens");

        index_repository(&store, root).expect("first index");
        let first_chunks = store.chunks_for_file("src/lib.rs").expect("chunks read");
        assert_eq!(first_chunks.len(), 1);
        assert_eq!(first_chunks[0].start_utf16, 0);
        assert_eq!(first_chunks[0].end_utf16, 5);
        assert_eq!(first_chunks[0].content, "a🚀文\n");

        fs::write(root.join("src/lib.rs"), "next\n").expect("modify lib");
        index_repository(&store, root).expect("second index");
        let second_chunks = store.chunks_for_file("src/lib.rs").expect("chunks read");
        assert_eq!(second_chunks.len(), 1);
        assert_eq!(second_chunks[0].start_utf16, 0);
        assert_eq!(second_chunks[0].end_utf16, 5);
        assert_eq!(second_chunks[0].content, "next\n");
        assert_ne!(first_chunks[0].id, second_chunks[0].id);

        fs::remove_file(root.join("src/lib.rs")).expect("delete lib");
        index_repository(&store, root).expect("third index");
        assert!(store
            .chunks_for_file("src/lib.rs")
            .expect("chunks read")
            .is_empty());
        assert_eq!(store.chunk_count().expect("chunk count"), 0);
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

    #[test]
    fn scan_repository_honors_custom_ignored_names() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join("vendor")).expect("vendor dir");
        fs::write(root.join("vendor/generated.rs"), "pub fn generated() {}\n")
            .expect("write generated");
        fs::write(root.join("src.rs"), "pub fn source() {}\n").expect("write source");
        let options = IndexOptions {
            ignored_names: vec![OsString::from("vendor")],
        };

        let files = scan_repository_with_options(root, &options).expect("scan repository");

        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["src.rs"])
        );
    }

    #[test]
    fn index_repository_updates_recovery_state_version_and_stats() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::write(root.join("src.rs"), "pub fn source() {}\n").expect("write source");
        let store = Store::open_in_memory().expect("store opens");

        let first = index_repository(&store, root).expect("first index");

        assert_eq!(first.added_files, 1);
        assert_eq!(
            store.index_recovery_state().expect("state reads"),
            crate::storage::IndexRecoveryState::Clean
        );
        assert_eq!(store.index_version().expect("version reads"), 1);
        assert_eq!(store.stat("index.total_runs").expect("total runs"), 1);
        assert_eq!(store.stat("index.scanned_files").expect("scanned files"), 1);
        assert_eq!(store.stat("index.added_files").expect("added files"), 1);

        let second = index_repository(&store, root).expect("second index");

        assert_eq!(second.unchanged_files, 1);
        assert_eq!(store.index_version().expect("version reads"), 2);
        assert_eq!(store.stat("index.total_runs").expect("total runs"), 2);
        assert_eq!(
            store
                .stat("index.unchanged_files")
                .expect("unchanged files"),
            1
        );
        assert_eq!(
            store
                .stat("index.skipped_embeddings")
                .expect("skipped embeddings"),
            1
        );
    }

    #[test]
    fn index_repository_marks_failed_state_when_scan_fails() {
        let temp = TempDir::new().expect("tempdir");
        let missing_root = temp.path().join("missing");
        let store = Store::open_in_memory().expect("store opens");

        let error = index_repository(&store, &missing_root).expect_err("index fails");

        assert!(error.to_string().contains("repository root does not exist"));
        assert_eq!(
            store.index_recovery_state().expect("state reads"),
            crate::storage::IndexRecoveryState::Failed
        );
        assert_eq!(store.index_version().expect("version reads"), 0);
        assert_eq!(store.stat("index.total_runs").expect("total runs"), 0);
        assert!(store
            .metadata("index.last_error")
            .expect("last error reads")
            .expect("last error set")
            .contains("repository root does not exist"));
    }
}
