#[derive(Debug, Clone, PartialEq)]
pub struct ChunkVector {
    pub chunk_id: String,
    pub file_path: String,
    pub chunk_index: u32,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchResult {
    pub chunk_id: String,
    pub file_path: String,
    pub chunk_index: u32,
    pub score: f32,
}

pub fn encode_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(embedding));
    for value in embedding {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub fn decode_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    let chunks = bytes.chunks_exact(std::mem::size_of::<f32>());
    if !chunks.remainder().is_empty() {
        return None;
    }

    Some(
        chunks
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }

    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        return None;
    }

    Some(dot / (left_norm.sqrt() * right_norm.sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_embedding_bytes() {
        let embedding = vec![0.25, -1.0, 3.5];

        assert_eq!(
            decode_embedding(&encode_embedding(&embedding)),
            Some(embedding)
        );
    }

    #[test]
    fn cosine_similarity_rejects_invalid_inputs() {
        assert_eq!(cosine_similarity(&[], &[]), None);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 0.0]), None);
        assert_eq!(cosine_similarity(&[0.0], &[1.0]), None);
    }
}
