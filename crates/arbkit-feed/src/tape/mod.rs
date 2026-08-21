//! High-performance binary market data tape recorder and deterministic player.

pub mod format;
pub mod player;
pub mod reader;
pub mod writer;

pub use format::{TapeHeader, TAPE_HEADER_SIZE, TAPE_MAGIC, TAPE_VERSION};
pub use player::TapePlayer;
pub use reader::TapeReader;
pub use writer::TapeWriter;
