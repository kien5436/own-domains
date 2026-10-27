mod message;
#[cfg(windows)]
pub mod pipe;
mod transport;

pub use message::*;
pub use transport::{Endpoint, MAX_FRAME, read_frame, write_json_line};
