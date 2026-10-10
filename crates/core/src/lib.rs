//! Porchly foundation: the error type, validated identifiers, field kinds and limits.
//! Everything above this crate speaks these types.

mod error;
mod ident;
mod kind;
pub mod limits;

pub use error::{Error, Result};
pub use ident::{Ident, RESERVED_NAMES, RESERVED_TABLES, SYSTEM_FIELDS, is_system_field};
pub use kind::FieldKind;
