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

#[macro_use]
extern crate futures;

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
    use futures::{Future, Sink, Stream};
    use std::net::SocketAddr;
    use std::thread;

    #[test]
    fn it_works() {
        let echo = tokio::net::TcpListener::bind(&SocketAddr::new("127.0.0.1".parse().unwrap(), 0))
            .unwrap();
        let addr = echo.local_addr().unwrap();

        let jh = thread::spawn(move || {
            tokio::run(
                echo.incoming()
                    .map_err(bincode::Error::from)
                    .take(1)
                    .for_each(|stream| {
                        let (r, w) = AsyncBincodeStream::<_, usize, usize, _>::from(stream)
                            .for_async()
                            .split();
                        r.forward(w).map(|_| ())
                    })
                    .map_err(|e| panic!(e)),
            )
        });

        let client = tokio::net::TcpStream::connect(&addr).wait().unwrap();
        let client = AsyncBincodeStream::from(client).for_async();
        let client = client.send(42usize).wait().unwrap();
        let (got, client) = match client.into_future().wait() {
            Ok(x) => x,
            Err((e, _)) => panic!(e),
        };
        assert_eq!(got, Some(42usize));

        let client = client.send(44usize).wait().unwrap();
        let (got, client) = match client.into_future().wait() {
            Ok(x) => x,
            Err((e, _)) => panic!(e),
        };
        assert_eq!(got, Some(44usize));

        drop(client);
        jh.join().unwrap();
    }

    #[test]
    fn it_works_spawn() {
        let echo = tokio::net::TcpListener::bind(&SocketAddr::new("127.0.0.1".parse().unwrap(), 0))
            .unwrap();
        let addr = echo.local_addr().unwrap();

        let jh = thread::spawn(move || {
            let mut rt = tokio::runtime::Runtime::new().unwrap();
            let (tx, rx) = futures::sync::mpsc::unbounded();

            let client = ::std::net::TcpStream::connect(&addr).unwrap();
            let client =
                tokio::net::TcpStream::from_std(client, &tokio::reactor::Handle::default())
                    .map(AsyncBincodeWriter::from)
                    .map(AsyncBincodeWriter::for_async)
                    .unwrap();

            rt.spawn(
                rx.forward(client.sink_map_err(|e| panic!("{:?}", e)))
                    .map(|_| ()),
            );
            tx.unbounded_send(42usize).unwrap();
            tx.unbounded_send(42usize).unwrap();
            rt.shutdown_on_idle();
        });

        tokio::run(
            echo.incoming()
                .map_err(bincode::Error::from)
                .take(1)
                .for_each(|stream| {
                    let (r, w) = AsyncBincodeStream::<_, usize, usize, _>::from(stream)
                        .for_async()
                        .split();
                    r.inspect(|&v| {
                        assert_eq!(v, 42usize);
                    })
                    .forward(w)
                    .map(|_| ())
                })
                .map_err(|e| panic!("{:?}", e)),
        );

        jh.join().unwrap();
    }

    #[test]
    fn lots() {
        let echo = tokio::net::TcpListener::bind(&SocketAddr::new("127.0.0.1".parse().unwrap(), 0))
            .unwrap();
        let addr = echo.local_addr().unwrap();

        let jh = thread::spawn(move || {
            tokio::run(
                echo.incoming()
                    .map_err(bincode::Error::from)
                    .take(1)
                    .for_each(|stream| {
                        let (r, w) = AsyncBincodeStream::<_, usize, usize, _>::from(stream)
                            .for_async()
                            .split();
                        r.forward(w).map(|_| ())
                    })
                    .map_err(|e| panic!(e)),
            )
        });

        let n = 81920;
        tokio::run(
            tokio::net::TcpStream::connect(&addr)
                .map_err(bincode::Error::from)
                .map(|stream| AsyncBincodeStream::from(stream).for_async())
                .and_then(move |c| futures::stream::iter_ok(0usize..n).forward(c))
                .and_then(|(_, mut c)| {
                    c.get_mut().shutdown(std::net::Shutdown::Write).unwrap();
                    c.fold(0, |at, got| -> Result<usize, bincode::Error> {
                        assert_eq!(at, got);
                        Ok(at + 1)
                    })
                })
                .map(move |v: usize| {
                    assert_eq!(v, n);
                })
                .map_err(|e| panic!("{:?}", e)),
        );
        jh.join().unwrap();
    }
}
