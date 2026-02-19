mod counts;
mod data;
pub(crate) mod fmt;
mod search;
mod session;
mod stats;

pub use search::{diff, export, query, watch};
pub use session::{run, sessions, tail};
pub use stats::{errors, stats_filtered, summary};

#[derive(Default)]
pub struct ViewArgs {
    pub last: Option<usize>,
    pub risk: Option<String>,
    pub tool: Option<String>,
    pub session: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub expand: bool,
}

const COLLAPSE_HEAD: usize = 5;
const COLLAPSE_TAIL: usize = 5;
const MAX_TABLE_ROWS: usize = 8;
