#[derive(Debug, Clone, PartialEq)]
pub struct FileFtsResult {
    pub path: String,
    pub language: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkFtsResult {
    pub chunk_id: String,
    pub file_path: String,
    pub chunk_index: u32,
    pub score: f64,
}
