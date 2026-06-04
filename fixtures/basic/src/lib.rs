pub struct SearchTarget {
    pub name: String,
}

impl SearchTarget {
    pub fn describe(&self) -> String {
        format!("target:{}", self.name)
    }
}

pub fn rank_score(exact_matches: u32, semantic_score: f32) -> f32 {
    exact_matches as f32 + semantic_score
}
