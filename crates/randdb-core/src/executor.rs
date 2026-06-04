use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::chunking::ChunkRecord;
use crate::contract::{
    CodebaseRetrievalRequest, CodebaseRetrievalResponse, CoreError, CoreRequest, CoreResponse,
    CoreResult, FindReferencesRequest, FindReferencesResponse, GetSymbolDefinitionRequest,
    GetSymbolDefinitionResponse, HealthRequest, IndexRepositoryRequest, IndexedFileEntry,
    ListFilesRequest, ListFilesResponse, LowConfidenceBehavior, OutputFormat, RetrievalDebug,
    RetrievalMode, RetrievalPack, StatsRequest, SymbolDefinition, SymbolReference,
};
use crate::indexer::index_repository;
use crate::packing::{pack_context, PackConfig};
use crate::search::{hybrid_search, HybridSearchInput, ScoredChunk, SearchConfig};
use crate::stats::{collect_health, collect_stats};
use crate::storage::{IndexedFile, Store};

pub fn execute_request(request: CoreRequest) -> CoreResult {
    match request {
        CoreRequest::IndexRepository(payload) => execute_index_repository(payload),
        CoreRequest::Stats(payload) => execute_stats(payload),
        CoreRequest::Health(payload) => execute_health(payload),
        CoreRequest::ListFiles(payload) => execute_list_files(payload),
        CoreRequest::CodebaseRetrieval(payload) => execute_codebase_retrieval(payload),
        CoreRequest::FindReferences(payload) => execute_find_references(payload),
        CoreRequest::GetSymbolDefinition(payload) => execute_get_symbol_definition(payload),
    }
}

fn execute_index_repository(payload: IndexRepositoryRequest) -> CoreResult {
    with_store(
        "index-repository",
        &payload.repo_path,
        payload.force_rebuild,
        |store| {
            let run = index_repository(store, &payload.repo_path)?;
            Ok(CoreResponse::IndexRepository(
                crate::contract::IndexRepositoryResponse {
                    project_id: project_id(&payload.repo_path),
                    run,
                },
            ))
        },
    )
}

fn execute_stats(payload: StatsRequest) -> CoreResult {
    with_store("stats", &payload.repo_path, false, |store| {
        Ok(CoreResponse::Stats(collect_stats(
            store,
            project_id(&payload.repo_path),
        )?))
    })
}

fn execute_health(payload: HealthRequest) -> CoreResult {
    with_store("health", &payload.repo_path, false, |store| {
        Ok(CoreResponse::Health(collect_health(
            store,
            project_id(&payload.repo_path),
        )?))
    })
}

fn execute_list_files(payload: ListFilesRequest) -> CoreResult {
    with_store("list-files", &payload.repo_path, false, |store| {
        let max_results = payload.max_results.unwrap_or(200);
        let files = store.list_files(None)?;
        let mut filtered = files
            .into_iter()
            .filter(|file| {
                payload
                    .language
                    .as_deref()
                    .is_none_or(|language| file.language == language)
            })
            .filter(|file| {
                payload
                    .glob
                    .as_deref()
                    .is_none_or(|glob| matches_glob(&file.path, glob))
            })
            .collect::<Vec<_>>();
        let truncated = filtered.len() > max_results as usize;
        filtered.truncate(max_results as usize);
        Ok(CoreResponse::ListFiles(ListFilesResponse {
            files: filtered.into_iter().map(indexed_file_entry).collect(),
            truncated,
        }))
    })
}

fn execute_codebase_retrieval(payload: CodebaseRetrievalRequest) -> CoreResult {
    with_store("codebase-retrieval", &payload.repo_path, false, |store| {
        let lexical_query = retrieval_query(&payload);
        let config = search_config(&payload);
        let mut output = hybrid_search(
            store,
            &HybridSearchInput {
                lexical_query: lexical_query.clone(),
                query_embedding: None,
                config: config.clone(),
            },
        )?;
        let languages = indexed_languages(store)?;
        let candidate_pool = if output.debug.low_confidence {
            output.seeds.clone()
        } else {
            output.candidates.clone()
        };
        let seeds = filter_scored_chunks(candidate_pool, &payload, &languages)
            .into_iter()
            .take(config.seed_top_k)
            .collect::<Vec<_>>();
        let pack = pack_context(store, &seeds, &pack_config(&payload))?;
        let packed_segment_count = count_segments(&pack);
        record_search_stats(store, seeds.len() as i64)?;
        let format = payload
            .output_format
            .clone()
            .unwrap_or(OutputFormat::Markdown);
        let markdown = match format {
            OutputFormat::Markdown | OutputFormat::Both => {
                Some(render_markdown(&pack, &output.warnings))
            }
            OutputFormat::Json => None,
        };
        let pack = match format {
            OutputFormat::Json | OutputFormat::Both => Some(pack),
            OutputFormat::Markdown => None,
        };
        let debug = if payload.return_debug {
            Some(RetrievalDebug {
                lexical_query,
                semantic_query: payload.information_request.clone(),
                candidate_count: output.debug.candidate_count,
                packed_segment_count,
            })
        } else {
            None
        };

        Ok(CoreResponse::CodebaseRetrieval(CodebaseRetrievalResponse {
            format,
            markdown,
            pack,
            debug,
            warnings: std::mem::take(&mut output.warnings),
        }))
    })
}

fn execute_find_references(payload: FindReferencesRequest) -> CoreResult {
    with_store("find-references", &payload.repo_path, false, |store| {
        let symbol = payload.symbol.clone();
        let max_results = payload.max_results.unwrap_or(50).min(200);
        let languages = indexed_languages(store)?;
        let search_limit = if payload.exclude_definition.unwrap_or(false) {
            200
        } else {
            max_results.saturating_add(1)
        };
        let mut references = symbol_references(store, &symbol, search_limit, &languages)?;
        if payload.exclude_definition.unwrap_or(false) {
            references.retain(|reference| !is_definition_line(&reference.snippet, &symbol));
        }
        let truncated = references.len() > max_results as usize;
        references.truncate(max_results as usize);
        Ok(CoreResponse::FindReferences(FindReferencesResponse {
            symbol,
            references,
            truncated,
        }))
    })
}

fn execute_get_symbol_definition(payload: GetSymbolDefinitionRequest) -> CoreResult {
    with_store(
        "get-symbol-definition",
        &payload.repo_path,
        false,
        |store| {
            let symbol = payload.symbol.clone();
            let max_results = payload.max_results.unwrap_or(3).min(20);
            let languages = indexed_languages(store)?;
            let mut definitions = symbol_definitions(
                store,
                &symbol,
                payload.hint_path.as_deref(),
                max_results.saturating_add(1),
                &languages,
            )?;
            let truncated = definitions.len() > max_results as usize;
            definitions.truncate(max_results as usize);
            Ok(CoreResponse::GetSymbolDefinition(
                GetSymbolDefinitionResponse {
                    symbol,
                    definitions,
                    truncated,
                },
            ))
        },
    )
}

fn with_store(
    operation_name: &'static str,
    repo_path: &str,
    force_rebuild: bool,
    operation: impl FnOnce(&Store) -> Result<CoreResponse, Box<dyn std::error::Error>>,
) -> CoreResult {
    let result = (|| {
        let db_path = database_path(repo_path);
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if force_rebuild && db_path.exists() {
            fs::remove_file(&db_path)?;
        }
        let store = Store::open(&db_path)?;
        operation(&store)
    })();

    match result {
        Ok(response) => CoreResult::Ok { response },
        Err(error) => CoreResult::Error {
            error: CoreError {
                code: "core_execution_error".to_string(),
                message: error.to_string(),
                operation: Some(operation_name.to_string()),
                retryable: false,
            },
        },
    }
}

fn database_path(repo_path: &str) -> PathBuf {
    Path::new(repo_path).join(".randdb").join("index.db")
}

fn project_id(repo_path: &str) -> String {
    Path::new(repo_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("randdb")
        .to_string()
}

fn indexed_file_entry(file: IndexedFile) -> IndexedFileEntry {
    IndexedFileEntry {
        path: file.path,
        language: file.language,
        size: file.size_bytes.max(0) as u64,
        hash: Some(file.hash),
    }
}

fn retrieval_query(payload: &CodebaseRetrievalRequest) -> String {
    if payload.technical_terms.is_empty() {
        payload.information_request.clone()
    } else {
        payload.technical_terms.join(" ")
    }
}

fn search_config(payload: &CodebaseRetrievalRequest) -> SearchConfig {
    let mut config = SearchConfig {
        w_vec: 0.0,
        w_lex: 1.0,
        low_confidence_behavior: payload
            .low_confidence_behavior
            .clone()
            .unwrap_or(LowConfidenceBehavior::ReturnTop1),
        ..SearchConfig::default()
    };

    match payload.mode.clone().unwrap_or(RetrievalMode::Balanced) {
        RetrievalMode::Quick => {
            config.lexical_top_k = 20;
            config.fused_top_m = 10;
            config.seed_top_k = 4;
        }
        RetrievalMode::Balanced => {}
        RetrievalMode::Deep => {
            config.lexical_top_k = 80;
            config.fused_top_m = 40;
            config.seed_top_k = 16;
        }
    }

    config
}

fn pack_config(payload: &CodebaseRetrievalRequest) -> PackConfig {
    let mut config = PackConfig::default();
    if let Some(max_total_chars) = payload.max_total_chars {
        config.max_total_chars = max_total_chars;
    }
    if let Some(max_files) = payload.max_files {
        config.max_files = max_files as usize;
    }
    if let Some(max_segments_per_file) = payload.max_segments_per_file {
        config.max_segments_per_file = max_segments_per_file as usize;
    }
    config.neighbor_radius = match payload.mode.clone().unwrap_or(RetrievalMode::Balanced) {
        RetrievalMode::Quick => 0,
        RetrievalMode::Balanced => 1,
        RetrievalMode::Deep => 2,
    };
    config
}

fn indexed_languages(store: &Store) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    Ok(store
        .list_files(None)?
        .into_iter()
        .map(|file| (file.path, file.language))
        .collect())
}

fn filter_scored_chunks(
    seeds: Vec<ScoredChunk>,
    payload: &CodebaseRetrievalRequest,
    languages: &HashMap<String, String>,
) -> Vec<ScoredChunk> {
    seeds
        .into_iter()
        .filter(|seed| include_path(&seed.chunk.file_path, payload))
        .filter(|seed| {
            payload.language.is_empty()
                || languages
                    .get(&seed.chunk.file_path)
                    .is_some_and(|language| {
                        payload.language.iter().any(|wanted| wanted == language)
                    })
        })
        .collect()
}

fn include_path(path: &str, payload: &CodebaseRetrievalRequest) -> bool {
    let included = payload.include_globs.is_empty()
        || payload
            .include_globs
            .iter()
            .any(|pattern| matches_glob(path, pattern));
    let excluded = payload
        .exclude_globs
        .iter()
        .any(|pattern| matches_glob(path, pattern));
    included && !excluded
}

fn count_segments(pack: &RetrievalPack) -> u32 {
    pack.files
        .iter()
        .map(|file| file.segments.len() as u32)
        .sum()
}

fn record_search_stats(
    store: &Store,
    recalled_seed_count: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    store.increment_stat("search.total_queries", 1)?;
    store.increment_stat("search.recalled_seed_count_total", recalled_seed_count)?;
    Ok(())
}

fn symbol_references(
    store: &Store,
    symbol: &str,
    limit: u32,
    languages: &HashMap<String, String>,
) -> Result<Vec<SymbolReference>, Box<dyn std::error::Error>> {
    let mut references = Vec::new();
    for chunk in chunks_matching_symbol(store, symbol)? {
        for (line_offset, line) in chunk.content.lines().enumerate() {
            if !contains_symbol(line, symbol) {
                continue;
            }
            references.push(SymbolReference {
                path: chunk.file_path.clone(),
                language: languages
                    .get(&chunk.file_path)
                    .cloned()
                    .unwrap_or_else(|| infer_language(&chunk.file_path).to_string()),
                start_line: chunk.start_line + line_offset as u32,
                end_line: chunk.start_line + line_offset as u32,
                snippet: line.trim().to_string(),
                breadcrumb: chunk.breadcrumb.clone(),
            });
            if references.len() >= limit as usize {
                return Ok(references);
            }
        }
    }
    Ok(references)
}

fn symbol_definitions(
    store: &Store,
    symbol: &str,
    hint_path: Option<&str>,
    limit: u32,
    languages: &HashMap<String, String>,
) -> Result<Vec<SymbolDefinition>, Box<dyn std::error::Error>> {
    let mut definitions = Vec::new();
    for chunk in chunks_matching_symbol(store, symbol)? {
        if hint_path.is_some_and(|hint| chunk.file_path != hint) {
            continue;
        }
        for (line_offset, line) in chunk.content.lines().enumerate() {
            if !contains_symbol(line, symbol) || !is_definition_line(line, symbol) {
                continue;
            }
            definitions.push(SymbolDefinition {
                path: chunk.file_path.clone(),
                language: languages
                    .get(&chunk.file_path)
                    .cloned()
                    .unwrap_or_else(|| infer_language(&chunk.file_path).to_string()),
                start_line: chunk.start_line + line_offset as u32,
                end_line: chunk.start_line + line_offset as u32,
                text: line.trim().to_string(),
                breadcrumb: chunk.breadcrumb.clone(),
            });
            if definitions.len() >= limit as usize {
                return Ok(definitions);
            }
        }
    }
    Ok(definitions)
}

fn chunks_matching_symbol(
    store: &Store,
    symbol: &str,
) -> Result<Vec<ChunkRecord>, Box<dyn std::error::Error>> {
    let mut chunks = Vec::new();
    let mut seen = HashSet::<String>::new();
    for result in store.search_chunks_fts(symbol, 200)? {
        if !seen.insert(result.chunk_id.clone()) {
            continue;
        }
        if let Some(chunk) = store.chunk_by_id(&result.chunk_id)? {
            chunks.push(chunk);
        }
    }

    if chunks.is_empty() {
        for path in store.all_file_paths()? {
            for chunk in store.chunks_for_file(&path)? {
                if chunk.content.contains(symbol) {
                    chunks.push(chunk);
                }
            }
        }
    }

    Ok(chunks)
}

fn contains_symbol(line: &str, symbol: &str) -> bool {
    if symbol.is_empty() {
        return false;
    }
    line.match_indices(symbol).any(|(index, _)| {
        let before = line[..index].chars().next_back();
        let after = line[index + symbol.len()..].chars().next();
        !before.is_some_and(is_identifier_char) && !after.is_some_and(is_identifier_char)
    })
}

fn is_identifier_char(char: char) -> bool {
    char == '_' || char.is_alphanumeric()
}

fn is_definition_line(line: &str, symbol: &str) -> bool {
    let trimmed = line.trim_start();
    let prefixes = [
        "fn ",
        "pub fn ",
        "async fn ",
        "pub async fn ",
        "struct ",
        "pub struct ",
        "enum ",
        "pub enum ",
        "trait ",
        "pub trait ",
        "type ",
        "pub type ",
        "const ",
        "pub const ",
        "let ",
        "export function ",
        "function ",
        "class ",
        "export class ",
        "interface ",
        "export interface ",
    ];
    prefixes.iter().any(|prefix| {
        trimmed.strip_prefix(prefix).is_some_and(|rest| {
            rest.trim_start()
                .strip_prefix(symbol)
                .is_some_and(|tail| !tail.chars().next().is_some_and(is_identifier_char))
        })
    })
}

fn infer_language(path: &str) -> &'static str {
    match path
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or_default()
    {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "md" | "markdown" => "markdown",
        "json" => "json",
        "toml" => "toml",
        _ => "text",
    }
}

fn render_markdown(pack: &RetrievalPack, warnings: &[String]) -> String {
    let mut out = String::new();
    let segment_count = count_segments(pack);
    writeln!(
        &mut out,
        "Found {} relevant code block{} | Files: {} | Total segments: {}",
        segment_count,
        if segment_count == 1 { "" } else { "s" },
        pack.files.len(),
        segment_count
    )
    .expect("writing into String cannot fail");

    for warning in warnings {
        writeln!(&mut out, "\nWarning: {warning}").expect("writing into String cannot fail");
    }

    for file in &pack.files {
        writeln!(&mut out, "\n## {} ({})", file.path, file.language)
            .expect("writing into String cannot fail");
        for segment in &file.segments {
            writeln!(
                &mut out,
                "\n```{}:{}-{}",
                file.language, segment.start_line, segment.end_line
            )
            .expect("writing into String cannot fail");
            out.push_str(&segment.text);
            if !segment.text.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("```\n");
        }
    }

    out
}

fn matches_glob(path: &str, pattern: &str) -> bool {
    matches_glob_at(path.as_bytes(), pattern.as_bytes())
}

fn matches_glob_at(path: &[u8], pattern: &[u8]) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }

    if let Some(rest) = pattern.strip_prefix(b"**") {
        return (0..=path.len()).any(|index| matches_glob_at(&path[index..], rest));
    }

    if let Some(rest) = pattern.strip_prefix(b"*") {
        return (0..=path
            .iter()
            .position(|byte| *byte == b'/')
            .unwrap_or(path.len()))
            .any(|index| matches_glob_at(&path[index..], rest));
    }

    if path.first() == pattern.first() {
        return matches_glob_at(&path[1..], &pattern[1..]);
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn index_then_list_stats_and_health_round_trip() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::write(root.join("src.rs"), "pub fn source() {}\n").expect("write source");

        let index_result = execute_request(CoreRequest::IndexRepository(IndexRepositoryRequest {
            repo_path: root.to_string_lossy().to_string(),
            force_rebuild: false,
        }));
        assert!(matches!(index_result, CoreResult::Ok { .. }));

        let list_result = execute_request(CoreRequest::ListFiles(ListFilesRequest {
            repo_path: root.to_string_lossy().to_string(),
            glob: Some("*.rs".to_string()),
            language: Some("rust".to_string()),
            max_results: Some(10),
        }));
        let CoreResult::Ok {
            response: CoreResponse::ListFiles(list),
        } = list_result
        else {
            panic!("list-files should succeed");
        };
        assert_eq!(list.files.len(), 1);
        assert_eq!(list.files[0].path, "src.rs");

        let stats_result = execute_request(CoreRequest::Stats(StatsRequest {
            repo_path: root.to_string_lossy().to_string(),
        }));
        assert!(matches!(stats_result, CoreResult::Ok { .. }));

        let health_result = execute_request(CoreRequest::Health(HealthRequest {
            repo_path: root.to_string_lossy().to_string(),
        }));
        assert!(matches!(health_result, CoreResult::Ok { .. }));
    }

    #[test]
    fn codebase_retrieval_returns_packed_markdown_and_debug() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::create_dir(root.join("src")).expect("src dir");
        fs::write(
            root.join("src").join("lib.rs"),
            "pub fn search_target() {}\npub fn other() {}\n",
        )
        .expect("write source");
        fs::write(root.join("notes.md"), "search_target in notes\n").expect("write notes");
        let root = root.to_string_lossy().to_string();

        assert!(matches!(
            execute_request(CoreRequest::IndexRepository(IndexRepositoryRequest {
                repo_path: root.clone(),
                force_rebuild: true,
            })),
            CoreResult::Ok { .. }
        ));

        let result = execute_request(CoreRequest::CodebaseRetrieval(CodebaseRetrievalRequest {
            repo_path: root.clone(),
            information_request: "Find search target".to_string(),
            technical_terms: vec!["search_target".to_string()],
            mode: Some(RetrievalMode::Quick),
            include_globs: vec!["src/*.rs".to_string()],
            exclude_globs: Vec::new(),
            language: vec!["rust".to_string()],
            max_total_chars: Some(2_000),
            max_files: Some(2),
            max_segments_per_file: Some(2),
            return_debug: true,
            low_confidence_behavior: Some(LowConfidenceBehavior::ReturnWithWarning),
            output_format: Some(OutputFormat::Both),
        }));

        let CoreResult::Ok {
            response: CoreResponse::CodebaseRetrieval(response),
        } = result
        else {
            panic!("codebase-retrieval should succeed");
        };
        assert!(response
            .markdown
            .as_deref()
            .expect("markdown")
            .contains("search_target"));
        assert_eq!(response.pack.expect("pack").files[0].path, "src/lib.rs");
        assert_eq!(
            response.debug.expect("debug").lexical_query,
            "search_target"
        );

        let CoreResult::Ok {
            response: CoreResponse::Stats(stats),
        } = execute_request(CoreRequest::Stats(StatsRequest { repo_path: root }))
        else {
            panic!("stats should succeed");
        };
        assert_eq!(stats.search.total_queries, 1);
        assert_eq!(stats.search.average_recall, Some(1.0));
    }

    #[test]
    fn symbol_tools_return_references_and_definition() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path();
        fs::create_dir(root.join("src")).expect("src dir");
        fs::write(
            root.join("src").join("lib.rs"),
            "pub fn source() {}\nfn caller() { source(); }\nfn sourceful() {}\n",
        )
        .expect("write source");
        let root = root.to_string_lossy().to_string();

        assert!(matches!(
            execute_request(CoreRequest::IndexRepository(IndexRepositoryRequest {
                repo_path: root.clone(),
                force_rebuild: true,
            })),
            CoreResult::Ok { .. }
        ));

        let CoreResult::Ok {
            response: CoreResponse::GetSymbolDefinition(definitions),
        } = execute_request(CoreRequest::GetSymbolDefinition(
            GetSymbolDefinitionRequest {
                repo_path: root.clone(),
                symbol: "source".to_string(),
                hint_path: Some("src/lib.rs".to_string()),
                max_results: Some(5),
            },
        ))
        else {
            panic!("get-symbol-definition should succeed");
        };
        assert_eq!(definitions.definitions.len(), 1);
        assert_eq!(definitions.definitions[0].text, "pub fn source() {}");

        let CoreResult::Ok {
            response: CoreResponse::FindReferences(references),
        } = execute_request(CoreRequest::FindReferences(FindReferencesRequest {
            repo_path: root,
            symbol: "source".to_string(),
            exclude_definition: Some(true),
            max_results: Some(10),
        }))
        else {
            panic!("find-references should succeed");
        };
        assert_eq!(references.references.len(), 1);
        assert_eq!(
            references.references[0].snippet,
            "fn caller() { source(); }"
        );
    }

    #[test]
    fn wildcard_glob_matches_common_patterns() {
        assert!(matches_glob("src/lib.rs", "src/*.rs"));
        assert!(matches_glob("src/lib.rs", "s*/lib.rs"));
        assert!(!matches_glob("src/lib.rs", "*.rs"));
        assert!(matches_glob("src/lib.rs", "src/*"));
        assert!(!matches_glob("src/lib.ts", "*.rs"));
        assert!(!matches_glob("src/dir/lib.rs", "src/*.rs"));
        assert!(matches_glob("src/dir/lib.rs", "src/**/*.rs"));
        assert!(matches_glob("src/dir/lib.rs", "**/*.rs"));
    }
}
