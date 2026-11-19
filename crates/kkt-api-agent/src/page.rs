use super::*;

pub type Result<T> = std::result::Result<T, RuleError>;

/// One page of a list, plus the cursor for the next page if there is one.
#[derive(Clone, Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}
