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

mod reader;
mod stream;
mod writer;

pub use crate::reader::AsyncBincodeReader;
pub use crate::stream::AsyncBincodeStream;
pub use crate::writer::AsyncBincodeWriter;
pub use crate::writer::{AsyncDestination, BincodeWriterFor, SyncDestination};

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
    let mut c = bincode::config();
    let c = c.limit(u32::max_value() as u64);
    let size = c.serialized_size(value)? as u32;
    writer.write_u32::<NetworkEndian>(size)?;
    c.serialize_into(writer, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::prelude::*;

    #[tokio::test]
    async fn it_works() {
        let mut echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = echo.local_addr().unwrap();

        tokio::spawn(
            async move {
                let (stream, _) = echo.accept().await.unwrap();
                let (r, w) = AsyncBincodeStream::<_, usize, usize, _>::from(stream)
                    .for_async()
                    .split();
                r.forward(w).await.unwrap();
            }
        );

        let client = tokio::net::TcpStream::connect(&addr).await.unwrap();
        let mut client = AsyncBincodeStream::<_, usize, usize, _>::from(client).for_async();
        client.send(42).await.unwrap();
        assert_eq!(client.next().await.unwrap().unwrap(), 42);

        client.send(44).await.unwrap();
        assert_eq!(client.next().await.unwrap().unwrap(), 44);

        drop(client);
    }

    #[tokio::test]
    async fn lots() {
        let mut echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = echo.local_addr().unwrap();

        tokio::spawn(
            async move {
                let (stream, _) = echo.accept().await.unwrap();
                let (r, w) = AsyncBincodeStream::<_, usize, usize, _>::from(stream)
                    .for_async()
                    .split();
                r.forward(w).await.unwrap();
            }
        );

        let n = 81920;
        let stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
        let mut c = AsyncBincodeStream::from(stream).for_async();

        futures::stream::iter(0usize..n).map(Ok).forward(&mut c).await.unwrap();

        tokio::net::tcp::TcpStream::shutdown(c.get_mut(), std::net::Shutdown::Write).unwrap();

        let mut at = 0;
        while let Some(got) = c.next().await.transpose().unwrap() {
            assert_eq!(at, got);
            at += 1;
        }
        assert_eq!(at, n);
    }
}
