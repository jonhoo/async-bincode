use crate::{AsyncBincodeReader, AsyncBincodeWriter};
use crate::{AsyncDestination, SyncDestination};
use futures_core::Stream;
use futures_sink::Sink;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{fmt, io};
use tokio::io::AsyncRead;

/// A wrapper around an asynchronous stream that receives and sends bincode-encoded values.
///
/// To use, provide a stream that implements both [`AsyncWrite`] and [`AsyncRead`], and then use
/// [`Sink`] to send values and [`Stream`] to receive them.
///
/// Note that an `AsyncBincodeStream` must be of the type [`AsyncDestination`] in order to be
/// compatible with an [`AsyncBincodeReader`] on the remote end (recall that it requires the
/// serialized size prefixed to the serialized data). The default is [`SyncDestination`], but these
/// can be easily toggled between using [`AsyncBincodeStream::for_async`].
#[derive(Debug)]
pub struct AsyncBincodeStream<S, R, W, D> {
    stream: AsyncBincodeReader<InternalAsyncWriter<S, W, D>, R>,
}

#[doc(hidden)]
pub struct InternalAsyncWriter<S, T, D>(AsyncBincodeWriter<S, T, D>);

impl<S: fmt::Debug, T, D> fmt::Debug for InternalAsyncWriter<S, T, D> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.get_ref().fmt(f)
    }
}

impl<S, R, W> Default for AsyncBincodeStream<S, R, W, SyncDestination>
where
    S: Default,
{
    fn default() -> Self {
        Self::from(S::default())
    }
}

impl<S, R, W, D> AsyncBincodeStream<S, R, W, D> {
    /// Gets a reference to the underlying stream.
    ///
    /// It is inadvisable to directly read from or write to the underlying stream.
    pub fn get_ref(&self) -> &S {
        &self.stream.get_ref().0.get_ref()
    }

    /// Gets a mutable reference to the underlying stream.
    ///
    /// It is inadvisable to directly read from or write to the underlying stream.
    pub fn get_mut(&mut self) -> &mut S {
        self.stream.get_mut().0.get_mut()
    }

    /// Unwraps this `AsyncBincodeStream`, returning the underlying stream.
    ///
    /// Note that any leftover serialized data that has not yet been sent, or received data that
    /// has not yet been deserialized, is lost.
    pub fn into_inner(self) -> S {
        self.stream.into_inner().0.into_inner()
    }
}

impl<S, R, W> From<S> for AsyncBincodeStream<S, R, W, SyncDestination> {
    fn from(stream: S) -> Self {
        AsyncBincodeStream {
            stream: AsyncBincodeReader::from(InternalAsyncWriter(AsyncBincodeWriter::from(stream))),
        }
    }
}

impl<S, R, W, D> AsyncBincodeStream<S, R, W, D> {
    /// Make this stream include the serialized data's size before each serialized value.
    ///
    /// This is necessary for compatability with a remote [`AsyncBincodeReader`].
    pub fn for_async(self) -> AsyncBincodeStream<S, R, W, AsyncDestination> {
        let stream = self.into_inner();
        AsyncBincodeStream {
            stream: AsyncBincodeReader::from(InternalAsyncWriter(
                AsyncBincodeWriter::from(stream).for_async(),
            )),
        }
    }

    /// Make this stream only send bincode-encoded values.
    ///
    /// This is necessary for compatability with stock `bincode` receivers.
    pub fn for_sync(self) -> AsyncBincodeStream<S, R, W, SyncDestination> {
        AsyncBincodeStream::from(self.into_inner())
    }
}

impl<R, W, D> AsyncBincodeStream<tokio::net::TcpStream, R, W, D> {
    /// Split a TCP-based stream into a read half and a write half.
    ///
    /// This is more performant than using a lock-based split like the one provided by `tokio-io`
    /// or `futures-util` since we know that reads and writes to a `TcpStream` can continue
    /// concurrently.
    ///
    /// Any partially sent or received state is preserved.
    pub fn tcp_split(
        &mut self,
    ) -> (
        AsyncBincodeReader<tokio::net::tcp::ReadHalf, R>,
        AsyncBincodeWriter<tokio::net::tcp::WriteHalf, W, D>,
    ) {
        // First, steal the reader state so it isn't lost
        let rbuff = self.stream.buffer.take();
        // Then, fish out the writer
        let writer = &mut self.stream.get_mut().0;
        // And steal the writer state so it isn't lost
        let wbuff = writer.buffer.split_off(0);
        let wsize = writer.written;
        // Now split the stream
        let (r, w) = writer.get_mut().split();
        // Then put the reader back together
        let mut reader = AsyncBincodeReader::from(r);
        reader.buffer = rbuff;
        // And then the writer
        let mut writer: AsyncBincodeWriter<_, _, D> = AsyncBincodeWriter::from(w).make_for();
        writer.buffer = wbuff;
        writer.written = wsize;
        // All good!
        (reader, writer)
    }
}

impl<S, T, D> AsyncRead for InternalAsyncWriter<S, T, D>
where
    S: AsyncRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(self.get_mut().get_mut()).poll_read(cx, buf)
    }
}

impl<S, T, D> Deref for InternalAsyncWriter<S, T, D> {
    type Target = AsyncBincodeWriter<S, T, D>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<S, T, D> DerefMut for InternalAsyncWriter<S, T, D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<S, R, W, D> Stream for AsyncBincodeStream<S, R, W, D>
where
    S: Unpin,
    AsyncBincodeReader<InternalAsyncWriter<S, W, D>, R>: Stream<Item = Result<R, bincode::Error>>,
{
    type Item = Result<R, bincode::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

impl<S, R, W, D> Sink<W> for AsyncBincodeStream<S, R, W, D>
where
    S: Unpin,
    AsyncBincodeWriter<S, W, D>: Sink<W, Error = bincode::Error>,
{
    type Error = bincode::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut **self.stream.get_mut()).poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: W) -> Result<(), Self::Error> {
        Pin::new(&mut **self.stream.get_mut()).start_send(item)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut **self.stream.get_mut()).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut **self.stream.get_mut()).poll_close(cx)
    }
}
