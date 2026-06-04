use std::collections::HashMap;

use rusqlite::Result;

use crate::contract::LowConfidenceBehavior;
use crate::storage::Store;
use crate::{ChunkFtsResult, ChunkRecord, VectorSearchResult};

#[derive(Debug, Clone, PartialEq)]
pub struct SearchConfig {
    pub vector_top_k: u32,
    pub lexical_top_k: u32,
    pub fused_top_m: usize,
    pub seed_top_k: usize,
    pub rrf_k: f32,
    pub w_vec: f32,
    pub w_lex: f32,
    pub min_score: f32,
    pub low_confidence_behavior: LowConfidenceBehavior,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            vector_top_k: 40,
            lexical_top_k: 40,
            fused_top_m: 20,
            seed_top_k: 8,
            rrf_k: 20.0,
            w_vec: 0.6,
            w_lex: 0.4,
            min_score: 0.02,
            low_confidence_behavior: LowConfidenceBehavior::ReturnTop1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HybridSearchInput {
    pub lexical_query: String,
    pub query_embedding: Option<Vec<f32>>,
    pub config: SearchConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredChunk {
    pub chunk: ChunkRecord,
    pub score: f32,
    pub vector_score: Option<f32>,
    pub lexical_score: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HybridSearchDebug {
    pub vector_count: u32,
    pub lexical_count: u32,
    pub candidate_count: u32,
    pub low_confidence: bool,
    pub w_vec: f32,
    pub w_lex: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HybridSearchOutput {
    pub seeds: Vec<ScoredChunk>,
    pub candidates: Vec<ScoredChunk>,
    pub debug: HybridSearchDebug,
    pub warnings: Vec<String>,
}

pub fn hybrid_search(store: &Store, input: &HybridSearchInput) -> Result<HybridSearchOutput> {
    let vector_results = match input.query_embedding.as_deref() {
        Some(embedding) => store.search_vectors(embedding, input.config.vector_top_k)?,
        None => Vec::new(),
    };
    let lexical_results = if input.lexical_query.trim().is_empty() {
        Vec::new()
    } else {
        store.search_chunks_fts(&input.lexical_query, input.config.lexical_top_k)?
    };

    let mut fused = fuse_candidates(store, &vector_results, &lexical_results, &input.config)?;
    fused.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.chunk.id.cmp(&right.chunk.id))
    });
    fused.truncate(input.config.fused_top_m);

    let mut warnings = Vec::new();
    let mut low_confidence = false;
    let mut seeds = fused
        .iter()
        .take(input.config.seed_top_k)
        .cloned()
        .collect::<Vec<_>>();

    if let Some(top) = fused.first() {
        if top.score < input.config.min_score {
            low_confidence = true;
            let warning = format!(
                "Low confidence: top fused score {:.3} is below threshold {:.3}.",
                top.score, input.config.min_score
            );
            match input.config.low_confidence_behavior {
                LowConfidenceBehavior::ReturnEmpty => {
                    warnings.push(warning);
                    seeds.clear();
                }
                LowConfidenceBehavior::ReturnWithWarning => {
                    warnings.push(warning);
                    seeds = vec![top.clone()];
                }
                LowConfidenceBehavior::ReturnTop1 => {
                    seeds = vec![top.clone()];
                }
            }
        }
    }

    Ok(HybridSearchOutput {
        seeds,
        candidates: fused.clone(),
        debug: HybridSearchDebug {
            vector_count: vector_results.len() as u32,
            lexical_count: lexical_results.len() as u32,
            candidate_count: fused.len() as u32,
            low_confidence,
            w_vec: input.config.w_vec,
            w_lex: input.config.w_lex,
        },
        warnings,
    })
}

fn fuse_candidates(
    store: &Store,
    vector_results: &[VectorSearchResult],
    lexical_results: &[ChunkFtsResult],
    config: &SearchConfig,
) -> Result<Vec<ScoredChunk>> {
    let mut by_chunk_id = HashMap::<String, CandidateScores>::new();

    for (rank, result) in vector_results.iter().enumerate() {
        let score = reciprocal_rank(rank, config.rrf_k) * config.w_vec;
        let candidate = by_chunk_id.entry(result.chunk_id.clone()).or_default();
        candidate.score += score;
        candidate.vector_score = Some(result.score);
    }

    for (rank, result) in lexical_results.iter().enumerate() {
        let score = reciprocal_rank(rank, config.rrf_k) * config.w_lex;
        let candidate = by_chunk_id.entry(result.chunk_id.clone()).or_default();
        candidate.score += score;
        candidate.lexical_score = Some(-result.score as f32);
    }

    let mut out = Vec::new();
    for (chunk_id, scores) in by_chunk_id {
        if let Some(chunk) = store.chunk_by_id(&chunk_id)? {
            out.push(ScoredChunk {
                chunk,
                score: scores.score,
                vector_score: scores.vector_score,
                lexical_score: scores.lexical_score,
            });
        }
    }
    Ok(out)
}

fn reciprocal_rank(rank: usize, k: f32) -> f32 {
    1.0 / (k + rank as f32 + 1.0)
}

#[derive(Debug, Default)]
struct CandidateScores {
    score: f32,
    vector_score: Option<f32>,
    lexical_score: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkVector, FileRecord};

    #[test]
    fn hybrid_search_fuses_vector_and_lexical_candidates() {
        let store = Store::open_in_memory().expect("store opens");
        seed_chunks(&store);
        store
            .replace_file_vectors(
                "src/lib.rs",
                &[
                    ChunkVector {
                        chunk_id: "src/lib.rs:hash:0".to_string(),
                        file_path: "src/lib.rs".to_string(),
                        chunk_index: 0,
                        embedding: vec![1.0, 0.0],
                    },
                    ChunkVector {
                        chunk_id: "src/lib.rs:hash:1".to_string(),
                        file_path: "src/lib.rs".to_string(),
                        chunk_index: 1,
                        embedding: vec![0.0, 1.0],
                    },
                ],
            )
            .expect("vectors replace");
        let input = HybridSearchInput {
            lexical_query: "SearchTarget".to_string(),
            query_embedding: Some(vec![0.0, 1.0]),
            config: SearchConfig::default(),
        };

        let output = hybrid_search(&store, &input).expect("hybrid search");

        assert_eq!(output.debug.vector_count, 2);
        assert_eq!(output.debug.lexical_count, 1);
        assert_eq!(output.seeds[0].chunk.id, "src/lib.rs:hash:0");
        assert!(output.seeds[0].vector_score.is_some());
        assert!(output.seeds[0].lexical_score.is_some());
        assert!(output
            .candidates
            .iter()
            .any(|candidate| candidate.chunk.id == "src/lib.rs:hash:1"));
    }

    #[test]
    fn low_confidence_behavior_can_return_empty() {
        let store = Store::open_in_memory().expect("store opens");
        seed_chunks(&store);
        let input = HybridSearchInput {
            lexical_query: "SearchTarget".to_string(),
            query_embedding: None,
            config: SearchConfig {
                min_score: 1.0,
                low_confidence_behavior: LowConfidenceBehavior::ReturnEmpty,
                ..SearchConfig::default()
            },
        };

        let output = hybrid_search(&store, &input).expect("hybrid search");

        assert!(output.seeds.is_empty());
        assert!(output.debug.low_confidence);
        assert_eq!(output.warnings.len(), 1);
    }

    fn seed_chunks(store: &Store) {
        let file = FileRecord {
            path: "src/lib.rs".to_string(),
            hash: "hash".to_string(),
            mtime_ms: 0,
            size_bytes: 0,
            content: "SearchTarget\nOtherSymbol\n".to_string(),
            language: "rust".to_string(),
        };
        store.upsert_file(&file).expect("file upserts");
        store
            .replace_file_chunks(
                "src/lib.rs",
                &[
                    ChunkRecord {
                        id: "src/lib.rs:hash:0".to_string(),
                        file_path: "src/lib.rs".to_string(),
                        chunk_index: 0,
                        start_line: 1,
                        end_line: 1,
                        start_utf16: 0,
                        end_utf16: 13,
                        breadcrumb: Some("SearchTarget".to_string()),
                        content: "SearchTarget\n".to_string(),
                    },
                    ChunkRecord {
                        id: "src/lib.rs:hash:1".to_string(),
                        file_path: "src/lib.rs".to_string(),
                        chunk_index: 1,
                        start_line: 2,
                        end_line: 2,
                        start_utf16: 13,
                        end_utf16: 25,
                        breadcrumb: Some("OtherSymbol".to_string()),
                        content: "OtherSymbol\n".to_string(),
                    },
                ],
            )
            .expect("chunks replace");
    }
}
