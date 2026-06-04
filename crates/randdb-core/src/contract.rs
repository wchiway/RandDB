use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetrievalMode {
    Quick,
    Balanced,
    Deep,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LowConfidenceBehavior {
    ReturnTop1,
    ReturnEmpty,
    ReturnWithWarning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Markdown,
    Json,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", content = "payload", rename_all = "kebab-case")]
pub enum CoreRequest {
    CodebaseRetrieval(CodebaseRetrievalRequest),
    ListFiles(ListFilesRequest),
    FindReferences(FindReferencesRequest),
    GetSymbolDefinition(GetSymbolDefinitionRequest),
    Stats(StatsRequest),
    IndexRepository(IndexRepositoryRequest),
    Health(HealthRequest),
}

impl CoreRequest {
    pub fn operation_name(&self) -> &'static str {
        match self {
            Self::CodebaseRetrieval(_) => "codebase-retrieval",
            Self::ListFiles(_) => "list-files",
            Self::FindReferences(_) => "find-references",
            Self::GetSymbolDefinition(_) => "get-symbol-definition",
            Self::Stats(_) => "stats",
            Self::IndexRepository(_) => "index-repository",
            Self::Health(_) => "health",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", content = "payload", rename_all = "kebab-case")]
pub enum CoreResponse {
    CodebaseRetrieval(CodebaseRetrievalResponse),
    ListFiles(ListFilesResponse),
    FindReferences(FindReferencesResponse),
    GetSymbolDefinition(GetSymbolDefinitionResponse),
    Stats(StatsResponse),
    IndexRepository(IndexRepositoryResponse),
    Health(HealthResponse),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CoreResult {
    Ok { response: CoreResponse },
    Error { error: CoreError },
}

impl CoreResult {
    pub fn unsupported(operation: impl Into<String>) -> Self {
        Self::Error {
            error: CoreError {
                code: "unsupported_operation".to_string(),
                message: "operation is not implemented by this RandDB core build".to_string(),
                operation: Some(operation.into()),
                retryable: false,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodebaseRetrievalRequest {
    pub repo_path: String,
    pub information_request: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub technical_terms: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<RetrievalMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_globs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_globs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub language: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_total_chars: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_files: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_segments_per_file: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub return_debug: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_confidence_behavior: Option<LowConfidenceBehavior>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_format: Option<OutputFormat>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodebaseRetrievalResponse {
    pub format: OutputFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack: Option<RetrievalPack>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<RetrievalDebug>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalPack {
    pub files: Vec<PackedFile>,
    pub char_count: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackedFile {
    pub path: String,
    pub language: String,
    pub segments: Vec<PackedSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackedSegment {
    pub start_line: u32,
    pub end_line: u32,
    pub start_utf16: u32,
    pub end_utf16: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breadcrumb: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalDebug {
    pub lexical_query: String,
    pub semantic_query: String,
    pub candidate_count: u32,
    pub packed_segment_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListFilesRequest {
    pub repo_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListFilesResponse {
    pub files: Vec<IndexedFileEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedFileEntry {
    pub path: String,
    pub language: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindReferencesRequest {
    pub repo_path: String,
    pub symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_definition: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindReferencesResponse {
    pub symbol: String,
    pub references: Vec<SymbolReference>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolReference {
    pub path: String,
    pub language: String,
    pub start_line: u32,
    pub end_line: u32,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breadcrumb: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSymbolDefinitionRequest {
    pub repo_path: String,
    pub symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSymbolDefinitionResponse {
    pub symbol: String,
    pub definitions: Vec<SymbolDefinition>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolDefinition {
    pub path: String,
    pub language: String,
    pub start_line: u32,
    pub end_line: u32,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breadcrumb: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsRequest {
    pub repo_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsResponse {
    pub project_id: String,
    pub health: HealthSnapshot,
    pub index: IndexStats,
    pub search: SearchStats,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthRequest {
    pub repo_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub project_id: String,
    pub health: HealthSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthSnapshot {
    pub state: HealthState,
    pub file_count: u64,
    pub chunk_count: u64,
    pub embedding_dimensions: Option<u32>,
    pub migration: MigrationSnapshot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationSnapshot {
    pub schema_version: u32,
    pub pending_migrations: u32,
    pub recovery_required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexStats {
    pub total_runs: u64,
    pub last_run: Option<IndexRunSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchStats {
    pub total_queries: u64,
    pub cache_hits: u64,
    pub average_retrieve_ms: f64,
    pub average_rerank_ms: f64,
    pub average_expand_ms: f64,
    pub average_pack_ms: f64,
    pub average_recall: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRepositoryRequest {
    pub repo_path: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub force_rebuild: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexRepositoryResponse {
    pub project_id: String,
    pub run: IndexRunSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexRunSummary {
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: u64,
    pub scanned_files: u64,
    pub unchanged_files: u64,
    pub added_files: u64,
    pub modified_files: u64,
    pub deleted_files: u64,
    pub embedded_chunks: u64,
    pub skipped_embeddings: u64,
    pub health_state: HealthState,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_codebase_retrieval_request_with_contextweaver_tool_name() {
        let request = CoreRequest::CodebaseRetrieval(CodebaseRetrievalRequest {
            repo_path: "/repo".to_string(),
            information_request: "Find retrieval flow".to_string(),
            technical_terms: vec!["SearchService".to_string()],
            mode: Some(RetrievalMode::Balanced),
            include_globs: Vec::new(),
            exclude_globs: vec!["target/**".to_string()],
            language: vec!["rust".to_string()],
            max_total_chars: Some(20_000),
            max_files: Some(5),
            max_segments_per_file: Some(2),
            return_debug: true,
            low_confidence_behavior: Some(LowConfidenceBehavior::ReturnWithWarning),
            output_format: Some(OutputFormat::Both),
        });

        let value = serde_json::to_value(request).expect("request serializes");

        assert_eq!(
            value,
            json!({
                "operation": "codebase-retrieval",
                "payload": {
                    "repo_path": "/repo",
                    "information_request": "Find retrieval flow",
                    "technical_terms": ["SearchService"],
                    "mode": "balanced",
                    "exclude_globs": ["target/**"],
                    "language": ["rust"],
                    "max_total_chars": 20000,
                    "max_files": 5,
                    "max_segments_per_file": 2,
                    "return_debug": true,
                    "low_confidence_behavior": "return_with_warning",
                    "output_format": "both"
                }
            })
        );
    }

    #[test]
    fn round_trips_find_references_request() {
        let json = r#"{
            "operation": "find-references",
            "payload": {
                "repo_path": "/repo",
                "symbol": "SearchTarget",
                "exclude_definition": true,
                "max_results": 50
            }
        }"#;

        let request: CoreRequest = serde_json::from_str(json).expect("request parses");

        assert_eq!(
            request,
            CoreRequest::FindReferences(FindReferencesRequest {
                repo_path: "/repo".to_string(),
                symbol: "SearchTarget".to_string(),
                exclude_definition: Some(true),
                max_results: Some(50),
            })
        );
        assert_eq!(request.operation_name(), "find-references");
    }

    #[test]
    fn serializes_degraded_stats_response() {
        let response = CoreResponse::Stats(StatsResponse {
            project_id: "project-test".to_string(),
            health: HealthSnapshot {
                state: HealthState::Degraded,
                file_count: 2,
                chunk_count: 4,
                embedding_dimensions: Some(1536),
                migration: MigrationSnapshot {
                    schema_version: 1,
                    pending_migrations: 1,
                    recovery_required: true,
                },
                diagnostics: vec!["recovery pending".to_string()],
            },
            index: IndexStats {
                total_runs: 1,
                last_run: None,
            },
            search: SearchStats {
                total_queries: 3,
                cache_hits: 1,
                average_retrieve_ms: 2.5,
                average_rerank_ms: 0.0,
                average_expand_ms: 0.0,
                average_pack_ms: 1.0,
                average_recall: None,
            },
            diagnostics: vec!["recovery pending".to_string()],
        });

        let value = serde_json::to_value(response).expect("response serializes");

        assert_eq!(value["operation"], "stats");
        assert_eq!(value["payload"]["health"]["state"], "degraded");
        assert_eq!(
            value["payload"]["health"]["migration"]["recovery_required"],
            true
        );
    }
}
