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
//!
//! This crate provides two sets of types in two separate modules. The types in the `futures`
//! module work with the `futures_io`/`async-std` ecosystem. The types in the `tokio` module work
//! with the `tokio` ecosystem.
#![deny(missing_docs)]
#[cfg(not(any(feature = "futures", feature = "tokio")))]
compile_error!("async-bincode: Enable at least one of \"futures\" or \"tokio\".");

#[macro_use]
mod reader;
#[macro_use]
mod stream;
#[macro_use]
mod writer;

#[cfg(feature = "futures")]
pub mod futures;
#[cfg(feature = "tokio")]
pub mod tokio;

pub use crate::writer::{AsyncDestination, BincodeWriterFor, SyncDestination};

#[cfg(all(test, feature = "futures"))]
mod futures_tests {
    use crate::futures::*;
    use ::futures::prelude::*;
    use ::macro_rules_attribute::apply;
    use ::smol_macros::test;

    #[apply(test!)]
    async fn it_works() {
        let echo = smol::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = echo.local_addr().unwrap();

        let server = smol::spawn(async move {
            let (stream, _) = echo.accept().await.unwrap();
            let mut stream = AsyncBincodeStream::<_, usize, usize, _>::from(stream).for_async();
            while let Some(item) = stream.next().await {
                stream.send(item.unwrap()).await.unwrap();
            }
        });

        let client = smol::net::TcpStream::connect(&addr).await.unwrap();
        let mut client = AsyncBincodeStream::<_, usize, usize, _>::from(client).for_async();
        client.send(42).await.unwrap();
        assert_eq!(client.next().await.unwrap().unwrap(), 42);

        client.send(44).await.unwrap();
        assert_eq!(client.next().await.unwrap().unwrap(), 44);

        drop(client);
        server.await;
    }

    #[apply(test!)]
    async fn lots() {
        let echo = smol::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = echo.local_addr().unwrap();

        let server = smol::spawn(async move {
            let (stream, _) = echo.accept().await.unwrap();
            let mut stream = AsyncBincodeStream::<_, usize, usize, _>::from(stream).for_async();
            while let Some(item) = stream.next().await {
                stream.send(item.unwrap()).await.unwrap();
            }
        });

        let n = 81920;
        let stream = smol::net::TcpStream::connect(&addr).await.unwrap();
        let mut c = AsyncBincodeStream::from(stream).for_async();

        ::futures::stream::iter(0usize..n)
            .map(Ok)
            .forward(&mut c)
            .await
            .unwrap();

        c.get_mut().shutdown(smol::net::Shutdown::Write).unwrap();

        let mut at = 0;
        while let Some(got) = c.next().await.transpose().unwrap() {
            assert_eq!(at, got);
            at += 1;
        }
        assert_eq!(at, n);

        server.await;
    }
}

#[cfg(all(test, feature = "tokio"))]
mod tokio_tests {
    use crate::tokio::*;
    use futures::prelude::*;
    use tokio::io::{AsyncWriteExt, BufStream};

    #[tokio::test]
    async fn it_works() {
        let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = echo.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = echo.accept().await.unwrap();
            let mut stream = AsyncBincodeStream::<_, usize, usize, _>::from(stream).for_async();
            let (r, w) = stream.tcp_split();
            r.map_err(Box::new)
                .map_err(Box::<dyn std::error::Error>::from)
                .forward(w.sink_map_err(Box::new).sink_err_into())
                .await
                .unwrap();
        });

        let client = tokio::net::TcpStream::connect(&addr).await.unwrap();
        let mut client = AsyncBincodeStream::<_, usize, usize, _>::from(client).for_async();
        client.send(42).await.unwrap();
        assert_eq!(client.next().await.unwrap().unwrap(), 42);

        client.send(44).await.unwrap();
        assert_eq!(client.next().await.unwrap().unwrap(), 44);

        drop(client);
    }

    #[test]
    fn sync_destination() {
        let echo = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = echo.local_addr().unwrap();

        let tokio = tokio::runtime::Runtime::new().unwrap();
        let server = tokio.spawn(async move {
            echo.set_nonblocking(true).unwrap();
            let echo = tokio::net::TcpListener::from_std(echo).unwrap();
            let (stream, _) = echo.accept().await.unwrap();
            let mut w = AsyncBincodeWriter::<_, i32, _>::from(stream);
            for i in 0..20 {
                w.send(i).await.unwrap();
            }
            w.close().await.unwrap();
        });

        let mut client = std::io::BufReader::new(std::net::TcpStream::connect(&addr).unwrap());
        for i in 0..20 {
            let goti: i32 =
                bincode::decode_from_reader(&mut client, bincode::config::standard()).unwrap();
            assert_eq!(goti, i);
        }

        // another read should now fail
        bincode::decode_from_reader::<i32, _, _>(&mut client, bincode::config::standard())
            .unwrap_err();
        drop(client);

        tokio.block_on(server).unwrap();
    }

    #[tokio::test]
    async fn lots() {
        let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = echo.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = echo.accept().await.unwrap();
            let stream =
                AsyncBincodeStream::<_, usize, usize, _>::from(BufStream::new(stream)).for_async();
            let (w, r) = stream.split();
            r.map_err(Box::new)
                .map_err(Box::<dyn std::error::Error>::from)
                .forward(w.sink_map_err(Box::new).sink_err_into())
                .await
                .unwrap();
        });

        let n = 81920;
        let stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
        let mut c = AsyncBincodeStream::from(BufStream::new(stream)).for_async();

        ::futures::stream::iter(0usize..n)
            .map(Ok)
            .forward(&mut c)
            .await
            .unwrap();

        let r = c.get_mut().shutdown().await;
        if !cfg!(target_os = "macos") {
            // https://github.com/tokio-rs/tokio/issues/4665
            r.unwrap();
        }

        let mut at = 0;
        while let Some(got) = c.next().await.transpose().unwrap() {
            assert_eq!(at, got);
            at += 1;
        }
        assert_eq!(at, n);
    }
}
