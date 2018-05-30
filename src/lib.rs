//! Asynchronous access to a bincode-encoded item stream.
//!
//! This crate enables you to asynchronously read from a bincode-encoded stream, or write
//! bincoded-encoded values. `bincode` does not support this natively, as it cannot easily [resume
//! from stream errors while encoding or decoding](https://github.com/TyOverby/bincode/issues/229).
//!
//! `async-bincode` works around that on the receive side by buffering received bytes until a full
//! element's worth of data has been received, and only then calling into bincode. To make this
//! work, it relies on the sender to prefix each encoded element with its encoded size. See
//! [`serialize_into`] for a convenience method that provides this.
//!
//! On the write side, `async-bincode` buffers the serialized values, and asynchronously sends the
//! resulting bytestream.
#![deny(missing_docs)]
#![deny(unused_extern_crates)]

extern crate bincode;
extern crate byteorder;
extern crate serde;
#[macro_use]
extern crate futures;
extern crate tokio;

mod reader;
mod stream;
mod writer;

pub use reader::AsyncBincodeReader;
pub use stream::AsyncBincodeStream;
pub use writer::AsyncBincodeWriter;
pub use writer::{AsyncDestination, BincodeWriterFor, SyncDestination};

use byteorder::{NetworkEndian, WriteBytesExt};

/// Serializes an object directly into a `Writer` using the default configuration.
///
/// If the serialization would take more bytes than allowed by the size limit, an error is returned
/// and no bytes will be written into the `Writer`.
pub fn serialize_into<W, T: ?Sized>(mut writer: W, value: &T) -> bincode::Result<()>
where
    W: std::io::Write,
    T: serde::Serialize,
{
    let c = bincode::config();
    let size = c.serialized_size(value)? as u32;
    writer.write_u32::<NetworkEndian>(size)?;
    c.serialize_into(writer, value)
}
