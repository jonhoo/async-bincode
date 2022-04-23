//! Asynchronous access to a bincode-encoded item stream.
//!
//! This crate enables you to asynchronously read from a bincode-encoded stream, or write
//! bincoded-encoded values. `bincode` does not support this natively, as it cannot easily [resume
//! from stream errors while encoding or decoding](https://github.com/TyOverby/bincode/issues/229).
//!
//! `async-bincode` works around that on the receive side by buffering received bytes until a full
//! element's worth of data has been received, and only then calling into bincode. To make this
//! work, it relies on the sender to prefix each encoded element with its encoded size.
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

#[cfg(all(test, feature = "tokio"))]
mod tokio_tests {
    use super::*;
    use futures::prelude::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn it_works() {
        let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = echo.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = echo.accept().await.unwrap();
            let mut stream = AsyncBincodeStream::<_, usize, usize, _>::from(stream).for_async();
            let (r, w) = stream.tcp_split();
            r.forward(w).await.unwrap();
        });

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
        let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = echo.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = echo.accept().await.unwrap();
            let mut stream = AsyncBincodeStream::<_, usize, usize, _>::from(stream).for_async();
            let (r, w) = stream.tcp_split();
            r.forward(w).await.unwrap();
        });

        let n = 81920;
        let stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
        let mut c = AsyncBincodeStream::from(stream).for_async();

        futures::stream::iter(0usize..n)
            .map(Ok)
            .forward(&mut c)
            .await
            .unwrap();

        c.get_mut().shutdown().await.unwrap();

        let mut at = 0;
        while let Some(got) = c.next().await.transpose().unwrap() {
            assert_eq!(at, got);
            at += 1;
        }
        assert_eq!(at, n);
    }
}
