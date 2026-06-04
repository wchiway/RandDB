use rusqlite::Result;

use crate::contract::{
    HealthResponse, HealthSnapshot, HealthState, IndexStats, MigrationSnapshot, SearchStats,
    StatsResponse,
};
use crate::storage::{IndexRecoveryState, Store, SCHEMA_VERSION};

pub fn collect_health(store: &Store, project_id: impl Into<String>) -> Result<HealthResponse> {
    Ok(HealthResponse {
        project_id: project_id.into(),
        health: build_health_snapshot(store)?,
    })
}

pub fn collect_stats(store: &Store, project_id: impl Into<String>) -> Result<StatsResponse> {
    let health = build_health_snapshot(store)?;
    let diagnostics = health.diagnostics.clone();

    Ok(StatsResponse {
        project_id: project_id.into(),
        health,
        index: IndexStats {
            total_runs: stat_u64(store, "index.total_runs")?,
            last_run: store.last_index_run()?,
        },
        search: SearchStats {
            total_queries: stat_u64(store, "search.total_queries")?,
            cache_hits: stat_u64(store, "search.cache_hits")?,
            average_retrieve_ms: average_ms(store, "search.retrieve_ms_total")?,
            average_rerank_ms: average_ms(store, "search.rerank_ms_total")?,
            average_expand_ms: average_ms(store, "search.expand_ms_total")?,
            average_pack_ms: average_ms(store, "search.pack_ms_total")?,
            average_recall: average_recall(store)?,
        },
        diagnostics,
    })
}

fn build_health_snapshot(store: &Store) -> Result<HealthSnapshot> {
    let recovery_state = store.index_recovery_state()?;
    let file_count = store.file_count()?;
    let chunk_count = store.chunk_count()?;
    let vector_count = store.vector_count()?;
    let mut diagnostics = Vec::new();

    match recovery_state {
        IndexRecoveryState::Clean => {}
        IndexRecoveryState::Running => diagnostics.push(
            "Index recovery state is running; a previous indexing pass may not have completed."
                .to_string(),
        ),
        IndexRecoveryState::Failed => {
            let last_error = store.metadata("index.last_error")?.unwrap_or_default();
            diagnostics.push(if last_error.is_empty() {
                "Index recovery state is failed.".to_string()
            } else {
                format!("Index recovery state is failed: {last_error}")
            });
        }
    }

    if chunk_count > 0 && vector_count == 0 {
        diagnostics.push(
            "Chunks are indexed but no vector rows exist; semantic retrieval is unavailable."
                .to_string(),
        );
    }

    let state = if recovery_state == IndexRecoveryState::Failed {
        HealthState::Unavailable
    } else if diagnostics.is_empty() {
        HealthState::Healthy
    } else {
        HealthState::Degraded
    };

    Ok(HealthSnapshot {
        state,
        file_count,
        chunk_count,
        embedding_dimensions: store.embedding_dimensions()?,
        migration: MigrationSnapshot {
            schema_version: schema_version(store)?,
            pending_migrations: 0,
            recovery_required: recovery_state != IndexRecoveryState::Clean,
        },
        diagnostics,
    })
}

fn schema_version(store: &Store) -> Result<u32> {
    Ok(store
        .metadata("schema_version")?
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(SCHEMA_VERSION))
}

fn stat_u64(store: &Store, key: &str) -> Result<u64> {
    Ok(store.stat(key)?.max(0) as u64)
}

fn average_ms(store: &Store, total_key: &str) -> Result<f64> {
    let total_queries = store.stat("search.total_queries")?;
    if total_queries <= 0 {
        return Ok(0.0);
    }

    Ok(store.stat(total_key)? as f64 / total_queries as f64)
}

fn average_recall(store: &Store) -> Result<Option<f64>> {
    let total_queries = store.stat("search.total_queries")?;
    if total_queries <= 0 {
        return Ok(None);
    }

    Ok(Some(
        store.stat("search.recalled_seed_count_total")? as f64 / total_queries as f64,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{index_repository, ChunkRecord, ChunkVector, FileRecord};
    use tempfile::TempDir;

    #[test]
    fn stats_report_includes_index_run_and_clean_health() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        std::fs::write(root.join("src.rs"), "pub fn source() {}\n").expect("write source");
        let store = Store::open_in_memory().expect("store opens");

        index_repository(&store, root).expect("index repository");
        let stats = collect_stats(&store, "project-test").expect("stats collect");

        assert_eq!(stats.project_id, "project-test");
        assert_eq!(stats.index.total_runs, 1);
        assert_eq!(stats.index.last_run.expect("last run").added_files, 1);
        assert_eq!(stats.health.file_count, 1);
        assert_eq!(stats.health.migration.schema_version, SCHEMA_VERSION);
        assert!(!stats.health.migration.recovery_required);
    }

    #[test]
    fn health_degrades_when_chunks_have_no_vectors() {
        let store = Store::open_in_memory().expect("store opens");
        seed_chunk_without_vector(&store);

        let health = collect_health(&store, "project-test")
            .expect("health collect")
            .health;

        assert_eq!(health.state, HealthState::Degraded);
        assert!(health
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("no vector rows")));
    }

    #[test]
    fn health_exposes_failed_recovery_state() {
        let store = Store::open_in_memory().expect("store opens");
        store.mark_index_failed("scan failed").expect("mark failed");

        let health = collect_health(&store, "project-test")
            .expect("health collect")
            .health;

        assert_eq!(health.state, HealthState::Unavailable);
        assert!(health.migration.recovery_required);
        assert!(health
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("scan failed")));
    }

    #[test]
    fn stats_report_uses_search_counters() {
        let store = Store::open_in_memory().expect("store opens");
        store
            .increment_stat("search.total_queries", 2)
            .expect("stat");
        store.increment_stat("search.cache_hits", 1).expect("stat");
        store
            .increment_stat("search.retrieve_ms_total", 10)
            .expect("stat");
        store
            .increment_stat("search.rerank_ms_total", 4)
            .expect("stat");
        store
            .increment_stat("search.expand_ms_total", 6)
            .expect("stat");
        store
            .increment_stat("search.pack_ms_total", 8)
            .expect("stat");
        store
            .increment_stat("search.recalled_seed_count_total", 6)
            .expect("stat");

        let stats = collect_stats(&store, "project-test").expect("stats collect");

        assert_eq!(stats.search.total_queries, 2);
        assert_eq!(stats.search.cache_hits, 1);
        assert_eq!(stats.search.average_retrieve_ms, 5.0);
        assert_eq!(stats.search.average_rerank_ms, 2.0);
        assert_eq!(stats.search.average_expand_ms, 3.0);
        assert_eq!(stats.search.average_pack_ms, 4.0);
        assert_eq!(stats.search.average_recall, Some(3.0));
    }

    #[test]
    fn health_reports_embedding_dimensions_when_vectors_exist() {
        let store = Store::open_in_memory().expect("store opens");
        seed_chunk_without_vector(&store);
        store
            .replace_file_vectors(
                "src/lib.rs",
                &[ChunkVector {
                    chunk_id: "src/lib.rs:hash:0".to_string(),
                    file_path: "src/lib.rs".to_string(),
                    chunk_index: 0,
                    embedding: vec![1.0, 0.0, 0.0],
                }],
            )
            .expect("vectors replace");

        let health = collect_health(&store, "project-test")
            .expect("health collect")
            .health;

        assert_eq!(health.embedding_dimensions, Some(3));
        assert!(health.diagnostics.is_empty());
    }

    fn seed_chunk_without_vector(store: &Store) {
        let file = FileRecord {
            path: "src/lib.rs".to_string(),
            hash: "hash".to_string(),
            mtime_ms: 0,
            size_bytes: 0,
            content: "pub fn source() {}\n".to_string(),
            language: "rust".to_string(),
        };
        store.upsert_file(&file).expect("file upserts");
        store
            .replace_file_chunks(
                "src/lib.rs",
                &[ChunkRecord {
                    id: "src/lib.rs:hash:0".to_string(),
                    file_path: "src/lib.rs".to_string(),
                    chunk_index: 0,
                    start_line: 1,
                    end_line: 1,
                    start_utf16: 0,
                    end_utf16: 19,
                    breadcrumb: Some("source".to_string()),
                    content: "pub fn source() {}\n".to_string(),
                }],
            )
            .expect("chunks replace");
    }
}
