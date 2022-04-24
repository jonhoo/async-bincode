use bincode::Options;
use byteorder::{ByteOrder, NetworkEndian};
use bytes::buf::Buf;
use bytes::BytesMut;
use futures_core::{ready, Stream};
use serde::Deserialize;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A wrapper around an asynchronous reader that produces an asynchronous stream of
/// bincode-decoded values.
///
/// To use, provide an async reader and then use [`Stream`] to access the deserialized values.
///
/// The async reader type should implement one of the following traits:
#[cfg_attr(feature = "futures", doc = "- [`futures_io::AsyncRead`]")]
#[cfg_attr(feature = "tokio", doc = "- [`tokio::io::AsyncRead`]")]
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

enum FillResult {
    Filled,
    EOF,
}

impl<R: Unpin, T> AsyncBincodeReader<R, T>
where
    for<'a> T: Deserialize<'a>,
{
    fn internal_poll_next<F>(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        poll_reader: F,
    ) -> Poll<Option<Result<T, bincode::Error>>>
    where
        F: Fn(Pin<&mut R>, &mut Context, &mut [u8]) -> Poll<Result<usize, io::Error>> + Copy,
    {
        if let FillResult::EOF = ready!(self
            .as_mut()
            .fill(cx, 5, poll_reader)
            .map_err(bincode::Error::from))?
        {
            return Poll::Ready(None);
        }

        let message_size: u32 = NetworkEndian::read_u32(&self.buffer[..4]);
        let target_buffer_size = message_size as usize;

        // since self.buffer.len() >= 4, we know that we can't get an clean EOF here
        ready!(self
            .as_mut()
            .fill(cx, target_buffer_size + 4, poll_reader)
            .map_err(bincode::Error::from))?;

        self.buffer.advance(4);
        let message = bincode::options()
            .with_limit(u32::max_value() as u64)
            .allow_trailing_bytes()
            .deserialize(&self.buffer[..target_buffer_size])?;
        self.buffer.advance(target_buffer_size);
        Poll::Ready(Some(Ok(message)))
    }

    fn fill<F>(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        target_size: usize,
        poll_reader: F,
    ) -> Poll<Result<FillResult, io::Error>>
    where
        F: Fn(Pin<&mut R>, &mut Context, &mut [u8]) -> Poll<Result<usize, io::Error>>,
    {
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
            let n = ready!(poll_reader(Pin::new(&mut self.reader), cx, &mut rest[..]))?;
            if n == 0 {
                if self.buffer.is_empty() {
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

#[cfg(feature = "futures")]
impl<R, T> Stream for AsyncBincodeReader<R, T>
where
    for<'a> T: Deserialize<'a>,
    R: futures_io::AsyncRead + Unpin,
{
    type Item = Result<T, bincode::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        self.internal_poll_next(cx, internal_poll_reader_futures)
    }
}

#[cfg(feature = "futures")]
fn internal_poll_reader_futures<R>(
    r: Pin<&mut R>,
    cx: &mut Context,
    rest: &mut [u8],
) -> Poll<Result<usize, io::Error>>
where
    R: futures_io::AsyncRead + Unpin,
{
    r.poll_read(cx, rest)
}

#[cfg(feature = "tokio")]
impl<R, T> Stream for AsyncBincodeReader<R, T>
where
    for<'a> T: Deserialize<'a>,
    R: tokio::io::AsyncRead + Unpin,
{
    type Item = Result<T, bincode::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        self.internal_poll_next(cx, internal_poll_reader_tokio)
    }
}

#[cfg(feature = "tokio")]
fn internal_poll_reader_tokio<R>(
    r: Pin<&mut R>,
    cx: &mut Context,
    rest: &mut [u8],
) -> Poll<Result<usize, io::Error>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = tokio::io::ReadBuf::new(rest);
    ready!(r.poll_read(cx, &mut buf))?;
    let n = buf.filled().len();
    Poll::Ready(Ok(n))
}
