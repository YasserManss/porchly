//! Caps on everything sized by input. Input is hostile: these keep a bad request from
//! exhausting memory or holding the database.

/// Longest table or field name.
pub const MAX_IDENT_LEN: usize = 63;
/// Most tables in one project.
pub const MAX_TABLES: usize = 1_000;
/// Most fields in one table.
pub const MAX_FIELDS: usize = 200;
/// Rows returned by a list when no limit is given.
pub const DEFAULT_PAGE: u32 = 50;
/// Most rows returned by one list.
pub const MAX_PAGE: u32 = 500;
/// Highest list offset.
pub const MAX_OFFSET: u64 = 100_000_000;
/// Most filters in one list request.
pub const MAX_FILTERS: usize = 32;
/// Most link fields expanded in one request.
pub const MAX_EXPAND: usize = 10;
/// Most values in one `in.` filter.
pub const MAX_IN_VALUES: usize = 100;
/// Largest CSV file that can be imported, in bytes.
pub const MAX_IMPORT_BYTES: u64 = 50 * 1024 * 1024;
/// Most rows in one import.
pub const MAX_IMPORT_ROWS: usize = 100_000;
/// Longest search text, in bytes.
pub const MAX_SEARCH_BYTES: usize = 500;
/// Largest text value, in bytes.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// Largest HTTP request body, in bytes.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Largest HTTP query string, in bytes.
pub const MAX_QUERY_BYTES: usize = 8 * 1024;
/// Longest API key name.
pub const MAX_KEY_NAME_LEN: usize = 100;
/// Snapshots kept per project; older ones are deleted.
pub const MAX_SNAPSHOTS: usize = 20;
/// Largest request body forwarded to a function, in bytes.
pub const MAX_FUNCTION_BODY_BYTES: usize = 10 * 1024 * 1024;
/// Largest function source file, in bytes.
pub const MAX_FUNCTION_CODE_BYTES: usize = 1024 * 1024;
/// Most functions in one project.
pub const MAX_FUNCTIONS: usize = 500;
/// Most variables one function can have.
pub const MAX_FUNCTION_VARS: usize = 100;
/// Most rows added by one bulk request to the API.
pub const MAX_BULK_ROWS: usize = 1_000;
/// Changes kept for undo; older ones are forgotten.
pub const MAX_UNDO_OPS: i64 = 200;
/// Days a change can be undone for.
pub const UNDO_DAYS: u32 = 7;
/// Daily backups kept per project.
pub const MAX_DAILY_BACKUPS: usize = 7;
