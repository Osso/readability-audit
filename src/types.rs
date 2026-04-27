use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub category: String,
    pub file: String,
    pub line: usize,
    pub function: Option<String>,
    pub problem: String,
    pub fix: String,
}
