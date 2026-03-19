use crate::search::Search;

use super::SearchHandle;

#[derive(Default)]
pub struct SearchState {
    pub query: Search,
    pub handle: Option<SearchHandle>,
}
