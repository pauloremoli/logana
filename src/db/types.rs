use serde::{Deserialize, Serialize};

/// A text comment attached to a group of log line indices.
/// The text may contain newlines for multi-line comments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub text: String,
    pub line_indices: Vec<usize>,
}
