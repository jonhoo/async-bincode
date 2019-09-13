use byteorder::{ByteOrder, NetworkEndian};
use bytes::BytesMut;
use futures_core::{ready, Stream};
use serde::Deserialize;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_io::AsyncRead;

/// A wrapper around an asynchronous reader that produces an asynchronous stream of
/// bincode-decoded values.
///
/// To use, provide a reader that implements [`AsyncRead`], and then use [`Stream`] to access the
/// deserialized values.
///
/// Note that the sender *must* prefix each serialized item with its size as reported by
/// `bincode::serialized_size` encoded as a four-byte network-endian encoded. See also
/// [`serialize_into`], which does this for you.
#[derive(Debug)]
pub struct AsyncBincodeReader<R, T> {
    reader: R,
    pub(crate) buffer: BytesMut,
    into: PhantomData<T>,
}

impl<R, T> Unpin for AsyncBincodeReader<R, T> where R: Unpin {}

impl<R, T> Default for AsyncBincodeReader<R, T>
where
    R: Default,
{
    fn default() -> Self {
        Self::from(R::default())
    }
}

impl<R, T> AsyncBincodeReader<R, T> {
    /// Gets a reference to the underlying reader.
    ///
    /// It is inadvisable to directly read from the underlying reader.
    pub fn get_ref(&self) -> &R {
        &self.reader
    }

    /// Gets a mutable reference to the underlying reader.
    ///
    /// It is inadvisable to directly read from the underlying reader.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.reader
    }

    /// Returns a reference to the internally buffered data.
    ///
    /// This will not attempt to fill the buffer if it is empty.
    pub fn buffer(&self) -> &[u8] {
        &self.buffer[..]
    }

    /// Unwraps this `AsyncBincodeReader`, returning the underlying reader.
    ///
    /// Note that any leftover data in the internal buffer is lost.
    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl<R, T> From<R> for AsyncBincodeReader<R, T> {
    fn from(reader: R) -> Self {
        AsyncBincodeReader {
            buffer: BytesMut::with_capacity(8192),
            reader,
            into: PhantomData,
        }
    }
}

impl<R, T> Stream for AsyncBincodeReader<R, T>
where
    for<'a> T: Deserialize<'a>,
    R: AsyncRead + Unpin,
{
    type Item = Result<T, bincode::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        if let FillResult::EOF = ready!(self.as_mut().fill(cx, 5).map_err(bincode::Error::from))? {
            return Poll::Ready(None);
        }

        let message_size: u32 = NetworkEndian::read_u32(&self.buffer[..4]);
        let target_buffer_size = message_size as usize;

        // since self.buffer.len() >= 4, we know that we can't get an clean EOF here
        ready!(self
            .as_mut()
            .fill(cx, target_buffer_size + 4)
            .map_err(bincode::Error::from))?;

        self.buffer.advance(4);
        let message = bincode::deserialize(&self.buffer[..target_buffer_size])?;
        self.buffer.advance(target_buffer_size);
        Poll::Ready(Some(Ok(message)))
    }
}

enum FillResult {
    Filled,
    EOF,
}

impl<R, T> AsyncBincodeReader<R, T>
where
    for<'a> T: Deserialize<'a>,
    R: AsyncRead + Unpin,
{
    fn fill(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        target_size: usize,
    ) -> Poll<Result<FillResult, io::Error>> {
        if self.buffer.len() >= target_size {
            // we already have the bytes we need!
            return Poll::Ready(Ok(FillResult::Filled));
        }

        // make sure we can fit all the data we're about to read
        // and then some, so we don't do a gazillion syscalls
        if self.buffer.capacity() < target_size {
            let missing = target_size - self.buffer.capacity();
            self.buffer.reserve(missing);
        }

        let had = self.buffer.len();
        // this is the bit we'll be reading into
        let mut rest = self.buffer.split_off(had);
        // this is safe because we're not extending beyond the reserved capacity
        // and we're never reading unwritten bytes
        let max = rest.capacity();
        unsafe { rest.set_len(max) };

        while self.buffer.len() < target_size {
            let n = ready!(Pin::new(&mut self.reader).poll_read(cx, &mut rest[..]))?;
            if n == 0 {
                if self.buffer.len() == 0 {
                    return Poll::Ready(Ok(FillResult::EOF));
                } else {
                    return Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)));
                }
            }

            // adopt the new bytes
            let read = rest.split_to(n);
            self.buffer.unsplit(read);
        }

        Poll::Ready(Ok(FillResult::Filled))
    }
}
