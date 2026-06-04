use crate::storage::FileRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRecord {
    pub id: String,
    pub file_path: String,
    pub chunk_index: u32,
    pub start_line: u32,
    pub end_line: u32,
    pub start_utf16: u32,
    pub end_utf16: u32,
    pub breadcrumb: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkConfig {
    pub max_utf16_units: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_utf16_units: 8_000,
        }
    }
}

pub fn chunk_file(record: &FileRecord, config: &ChunkConfig) -> Vec<ChunkRecord> {
    if record.content.is_empty() || record.language == "binary" {
        return Vec::new();
    }

    let max_utf16_units = config.max_utf16_units.max(1);
    let mut chunks = Vec::new();
    let mut current = PendingChunk::new(0, 1);

    for line in split_lines_preserving_endings(&record.content) {
        let line_utf16 = utf16_len(line);
        if !current.content.is_empty() && current.utf16_len + line_utf16 > max_utf16_units {
            let next_start_utf16 = current.end_utf16;
            let next_start_line = current.end_line + 1;
            chunks.push(current.finish(record, chunks.len() as u32));
            current = PendingChunk::new(next_start_utf16, next_start_line);
        }

        current.push_line(line);
    }

    if !current.content.is_empty() {
        chunks.push(current.finish(record, chunks.len() as u32));
    }

    chunks
}

fn split_lines_preserving_endings(content: &str) -> impl Iterator<Item = &str> {
    content.split_inclusive('\n')
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

#[derive(Debug)]
struct PendingChunk {
    content: String,
    utf16_len: usize,
    start_utf16: u32,
    end_utf16: u32,
    start_line: u32,
    end_line: u32,
}

impl PendingChunk {
    fn new(start_utf16: u32, start_line: u32) -> Self {
        Self {
            content: String::new(),
            utf16_len: 0,
            start_utf16,
            end_utf16: start_utf16,
            start_line,
            end_line: start_line.saturating_sub(1),
        }
    }

    fn push_line(&mut self, line: &str) {
        let line_utf16 = utf16_len(line);
        self.content.push_str(line);
        self.utf16_len += line_utf16;
        self.end_utf16 = self.end_utf16.saturating_add(line_utf16 as u32);
        self.end_line = self.end_line.saturating_add(1);
    }

    fn finish(self, record: &FileRecord, chunk_index: u32) -> ChunkRecord {
        ChunkRecord {
            id: format!("{}:{}:{}", record.path, record.hash, chunk_index),
            file_path: record.path.clone(),
            chunk_index,
            start_line: self.start_line,
            end_line: self.end_line,
            start_utf16: self.start_utf16,
            end_utf16: self.end_utf16,
            breadcrumb: Some(record.path.clone()),
            content: self.content,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(content: &str) -> FileRecord {
        FileRecord {
            path: "src/lib.rs".to_string(),
            hash: "hash".to_string(),
            mtime_ms: 0,
            size_bytes: content.len() as i64,
            content: content.to_string(),
            language: "rust".to_string(),
        }
    }

    #[test]
    fn chunks_small_file_as_single_utf16_span() {
        let chunks = chunk_file(&record("a🚀文\n"), &ChunkConfig::default());

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_utf16, 0);
        assert_eq!(chunks[0].end_utf16, 5);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 1);
        assert_eq!(chunks[0].content, "a🚀文\n");
    }

    #[test]
    fn splits_on_line_boundaries_when_budget_is_exceeded() {
        let chunks = chunk_file(
            &record("one\ntwo\nthree\n"),
            &ChunkConfig { max_utf16_units: 8 },
        );

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].content, "one\ntwo\n");
        assert_eq!(chunks[0].start_utf16, 0);
        assert_eq!(chunks[0].end_utf16, 8);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 2);
        assert_eq!(chunks[1].content, "three\n");
        assert_eq!(chunks[1].start_utf16, 8);
        assert_eq!(chunks[1].end_utf16, 14);
        assert_eq!(chunks[1].start_line, 3);
        assert_eq!(chunks[1].end_line, 3);
    }

    #[test]
    fn binary_files_do_not_produce_chunks() {
        let mut binary = record("");
        binary.language = "binary".to_string();

        assert!(chunk_file(&binary, &ChunkConfig::default()).is_empty());
    }
}
