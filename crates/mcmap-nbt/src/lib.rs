//! Java NBT used by mcmap: checked byte parsing, owned arrays and Serde models.
#![forbid(unsafe_code)]
mod access;
mod arrays;
pub mod borrow;
mod cursor;
mod de;
pub mod error;
mod macros;
mod ser;
mod skip;
mod value;
mod write;
pub use arrays::{ByteArray, IntArray, LongArray};
pub use cursor::Cursor;
pub use de::from_bytes;
pub use error::{Error, Result};
pub use value::Value;
pub use write::to_bytes;
// Keep the historical Serde array identifiers so compounds cannot impersonate arrays.
const TOKENS: [&str; 3] = [
    "__fastnbt_byte_array",
    "__fastnbt_int_array",
    "__fastnbt_long_array",
];
