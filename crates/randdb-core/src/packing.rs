use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Result;

use crate::contract::{PackedFile, PackedSegment, RetrievalPack};
use crate::search::ScoredChunk;
use crate::storage::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackConfig {
    pub max_total_chars: u32,
    pub max_files: usize,
    pub max_segments_per_file: usize,
    pub neighbor_radius: u32,
}

impl Default for PackConfig {
    fn default() -> Self {
        Self {
            max_total_chars: 20_000,
            max_files: 8,
            max_segments_per_file: 4,
            neighbor_radius: 1,
        }
    }
}

pub fn pack_context(
    store: &Store,
    seeds: &[ScoredChunk],
    config: &PackConfig,
) -> Result<RetrievalPack> {
    let expanded = expand_context(store, seeds, config.neighbor_radius)?;
    let mut by_file = BTreeMap::<String, Vec<PackedSegment>>::new();
    let mut file_languages = BTreeMap::<String, String>::new();
    let mut total_utf16 = 0_u32;
    let mut truncated = false;

    for scored in expanded {
        if by_file.len() >= config.max_files && !by_file.contains_key(&scored.chunk.file_path) {
            truncated = true;
            continue;
        }

        let segments = by_file.entry(scored.chunk.file_path.clone()).or_default();
        if segments.len() >= config.max_segments_per_file {
            truncated = true;
            continue;
        }

        let Some(content) = store.file_content(&scored.chunk.file_path)? else {
            truncated = true;
            continue;
        };
        let Some(text) = slice_utf16(&content, scored.chunk.start_utf16, scored.chunk.end_utf16)
        else {
            truncated = true;
            continue;
        };
        let segment_utf16 = utf16_len(&text) as u32;
        if total_utf16.saturating_add(segment_utf16) > config.max_total_chars {
            truncated = true;
            break;
        }

        file_languages
            .entry(scored.chunk.file_path.clone())
            .or_insert_with(|| infer_language(&scored.chunk.file_path).to_string());
        total_utf16 += segment_utf16;
        segments.push(PackedSegment {
            start_line: scored.chunk.start_line,
            end_line: scored.chunk.end_line,
            start_utf16: scored.chunk.start_utf16,
            end_utf16: scored.chunk.end_utf16,
            breadcrumb: scored.chunk.breadcrumb,
            text,
            score: Some(scored.score),
        });
    }

    let files = by_file
        .into_iter()
        .map(|(path, segments)| PackedFile {
            language: file_languages
                .remove(&path)
                .unwrap_or_else(|| infer_language(&path).to_string()),
            path,
            segments,
        })
        .collect();

    Ok(RetrievalPack {
        files,
        char_count: total_utf16,
        truncated,
    })
}

pub fn expand_context(
    store: &Store,
    seeds: &[ScoredChunk],
    neighbor_radius: u32,
) -> Result<Vec<ScoredChunk>> {
    let seed_scores = seeds
        .iter()
        .map(|seed| (seed.chunk.id.clone(), seed.score))
        .collect::<HashMap<_, _>>();
    let mut out = Vec::new();
    let mut seen = HashSet::<String>::new();

    for seed in seeds {
        let file_chunks = store.chunks_for_file(&seed.chunk.file_path)?;
        let center = seed.chunk.chunk_index as i64;
        let radius = i64::from(neighbor_radius);
        for chunk in file_chunks {
            let distance = (i64::from(chunk.chunk_index) - center).abs();
            if distance > radius {
                continue;
            }

            if seen.insert(chunk.id.clone()) {
                let score = seed_scores
                    .get(&chunk.id)
                    .copied()
                    .unwrap_or(seed.score * 0.5);
                out.push(ScoredChunk {
                    chunk,
                    score,
                    vector_score: None,
                    lexical_score: None,
                });
            } else if let Some(seed_score) = seed_scores.get(&chunk.id) {
                if let Some(existing) = out.iter_mut().find(|scored| scored.chunk.id == chunk.id) {
                    existing.score = existing.score.max(*seed_score);
                }
            }
        }
    }

    out.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.chunk.file_path.cmp(&right.chunk.file_path))
            .then_with(|| left.chunk.chunk_index.cmp(&right.chunk.chunk_index))
    });
    Ok(out)
}

fn slice_utf16(content: &str, start_utf16: u32, end_utf16: u32) -> Option<String> {
    if start_utf16 > end_utf16 {
        return None;
    }

    let start = byte_index_for_utf16(content, start_utf16)?;
    let end = byte_index_for_utf16(content, end_utf16)?;
    content.get(start..end).map(ToString::to_string)
}

fn byte_index_for_utf16(content: &str, offset: u32) -> Option<usize> {
    let mut current = 0_u32;
    for (byte_index, char) in content.char_indices() {
        if current == offset {
            return Some(byte_index);
        }
        current = current.saturating_add(char.len_utf16() as u32);
        if current > offset {
            return None;
        }
    }

    if current == offset {
        Some(content.len())
    } else {
        None
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkRecord, FileRecord};

    #[test]
    fn packs_authoritative_utf16_slices_from_file_content() {
        let store = Store::open_in_memory().expect("store opens");
        seed_file(
            &store,
            "src/lib.rs",
            "a🚀文\nnext\n",
            &[
                ChunkRecord {
                    id: "src/lib.rs:hash:0".to_string(),
                    file_path: "src/lib.rs".to_string(),
                    chunk_index: 0,
                    start_line: 1,
                    end_line: 1,
                    start_utf16: 0,
                    end_utf16: 5,
                    breadcrumb: Some("emoji_width_marker".to_string()),
                    content: "stale chunk text".to_string(),
                },
                ChunkRecord {
                    id: "src/lib.rs:hash:1".to_string(),
                    file_path: "src/lib.rs".to_string(),
                    chunk_index: 1,
                    start_line: 2,
                    end_line: 2,
                    start_utf16: 5,
                    end_utf16: 10,
                    breadcrumb: Some("next".to_string()),
                    content: "next\n".to_string(),
                },
            ],
        );
        let seed = ScoredChunk {
            chunk: store
                .chunk_by_id("src/lib.rs:hash:0")
                .expect("chunk reads")
                .expect("chunk exists"),
            score: 1.0,
            vector_score: None,
            lexical_score: None,
        };

        let pack = pack_context(
            &store,
            &[seed],
            &PackConfig {
                max_total_chars: 10,
                max_files: 1,
                max_segments_per_file: 2,
                neighbor_radius: 1,
            },
        )
        .expect("packs");

        assert_eq!(pack.char_count, 10);
        assert!(!pack.truncated);
        assert_eq!(pack.files[0].segments[0].text, "a🚀文\n");
        assert_eq!(pack.files[0].segments[0].end_utf16, 5);
        assert_eq!(pack.files[0].segments[1].text, "next\n");
    }

    #[test]
    fn packing_enforces_budget_and_segment_limits() {
        let store = Store::open_in_memory().expect("store opens");
        seed_file(
            &store,
            "src/lib.rs",
            "first\nsecond\nthird\n",
            &[
                chunk("src/lib.rs", 0, 0, 6, "first\n"),
                chunk("src/lib.rs", 1, 6, 13, "second\n"),
                chunk("src/lib.rs", 2, 13, 19, "third\n"),
            ],
        );
        let seed = ScoredChunk {
            chunk: store
                .chunk_by_id("src/lib.rs:hash:1")
                .expect("chunk reads")
                .expect("chunk exists"),
            score: 1.0,
            vector_score: None,
            lexical_score: None,
        };

        let pack = pack_context(
            &store,
            &[seed],
            &PackConfig {
                max_total_chars: 12,
                max_files: 1,
                max_segments_per_file: 1,
                neighbor_radius: 2,
            },
        )
        .expect("packs");

        assert!(pack.truncated);
        assert_eq!(pack.files[0].segments.len(), 1);
        assert!(pack.char_count <= 12);
    }

    #[test]
    fn expansion_promotes_seed_score_when_seed_was_first_added_as_neighbor() {
        let store = Store::open_in_memory().expect("store opens");
        seed_file(
            &store,
            "src/lib.rs",
            "first\nsecond\n",
            &[
                chunk("src/lib.rs", 0, 0, 6, "first\n"),
                chunk("src/lib.rs", 1, 6, 13, "second\n"),
            ],
        );
        let first = ScoredChunk {
            chunk: store
                .chunk_by_id("src/lib.rs:hash:0")
                .expect("chunk reads")
                .expect("chunk exists"),
            score: 0.8,
            vector_score: None,
            lexical_score: None,
        };
        let second = ScoredChunk {
            chunk: store
                .chunk_by_id("src/lib.rs:hash:1")
                .expect("chunk reads")
                .expect("chunk exists"),
            score: 1.0,
            vector_score: None,
            lexical_score: None,
        };

        let expanded = expand_context(&store, &[first, second], 1).expect("expands");

        assert_eq!(expanded[0].chunk.id, "src/lib.rs:hash:1");
        assert_eq!(expanded[0].score, 1.0);
    }

    fn seed_file(store: &Store, path: &str, content: &str, chunks: &[ChunkRecord]) {
        let file = FileRecord {
            path: path.to_string(),
            hash: "hash".to_string(),
            mtime_ms: 0,
            size_bytes: content.len() as i64,
            content: content.to_string(),
            language: infer_language(path).to_string(),
        };
        store.upsert_file(&file).expect("file upserts");
        store
            .replace_file_chunks(path, chunks)
            .expect("chunks replace");
    }

    fn chunk(
        path: &str,
        index: u32,
        start_utf16: u32,
        end_utf16: u32,
        content: &str,
    ) -> ChunkRecord {
        ChunkRecord {
            id: format!("{path}:hash:{index}"),
            file_path: path.to_string(),
            chunk_index: index,
            start_line: index + 1,
            end_line: index + 1,
            start_utf16,
            end_utf16,
            breadcrumb: Some(format!("chunk_{index}")),
            content: content.to_string(),
        }
    }
}
