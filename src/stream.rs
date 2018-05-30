use bincode;
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};
use std::{fmt, io};
use tokio;
use tokio::prelude::*;
use {AsyncBincodeReader, AsyncBincodeWriter};
use {AsyncDestination, BincodeWriterFor, SyncDestination};

/// A wrapper around an asynchronous stream that receives and sends bincode-encoded values.
///
/// To use, provide a stream that implements both `tokio::io::AsyncWrite` and
/// `tokio::io::AsyncRead`, and then use `futures::Sink` to send values and `futures::Stream` to
/// receive them.
///
/// Note that an `AsyncBincodeStream` must be of the type [`AsyncDestination`] in order to be
/// compatible with an [`AsyncBincodeReader`] on the remote end (recall that it requires the
/// serialized size prefixed to the serialized data). The default is [`SyncDestination`], but these
/// can be easily toggled between using [`AsyncBincodeStream::for_async`].
#[derive(Debug)]
pub struct AsyncBincodeStream<S, R, W, D> {
    stream: AsyncBincodeReader<InternalAsyncWriter<S, W, D>, R>,
}

struct InternalAsyncWriter<S, T, D>(AsyncBincodeWriter<S, T, D>);

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

    /// Split this async stream into a write half and a read half.
    ///
    /// Any partially sent or received state is preserved.
    pub fn split(mut self) -> (AsyncBincodeWriter<S, W, D>, AsyncBincodeReader<S, R>)
    where
        S: Clone,
    {
        // First, steal the reader state so it isn't lost
        let rbuff = self.stream.buffer.split_off(0);
        let size = self.stream.size;
        // Then, fish out the writer
        let writer = self.stream.into_inner().0;
        // Now clone the stream for the reader
        let stream = writer.get_ref().clone();
        // Then put the reader back together
        let mut reader = AsyncBincodeReader::from(stream);
        reader.buffer = rbuff;
        reader.size = size;
        // All good!
        (writer, reader)
    }
}

impl<S, T, D> tokio::io::AsyncRead for InternalAsyncWriter<S, T, D>
where
    S: tokio::io::AsyncRead,
{
}
impl<S, T, D> io::Read for InternalAsyncWriter<S, T, D>
where
    S: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.get_mut().read(buf)
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
    for<'a> R: Deserialize<'a>,
    S: tokio::io::AsyncRead,
{
    type Item = R;
    type Error = bincode::Error;
    fn poll(&mut self) -> Result<Async<Option<Self::Item>>, Self::Error> {
        self.stream.poll()
    }
}

impl<S, R, W, D> Sink for AsyncBincodeStream<S, R, W, D>
where
    W: Serialize,
    S: tokio::io::AsyncWrite,
    AsyncBincodeWriter<S, W, D>: BincodeWriterFor<W>,
{
    type SinkItem = W;
    type SinkError = bincode::Error;

    fn start_send(
        &mut self,
        item: Self::SinkItem,
    ) -> Result<AsyncSink<Self::SinkItem>, Self::SinkError> {
        self.stream.get_mut().start_send(item)
    }

    fn poll_complete(&mut self) -> Result<Async<()>, Self::SinkError> {
        self.stream.get_mut().poll_complete()
    }
}
