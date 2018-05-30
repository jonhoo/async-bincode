use bincode;
use byteorder::{ByteOrder, NetworkEndian};
use serde::Deserialize;
use std::io;
use std::marker::PhantomData;
use tokio;
use tokio::prelude::*;

/// A wrapper around an asynchronous reader that produces an asynchronous stream of
/// bincode-decoded values.
///
/// To use, provide a reader that implements `tokio::io::AsyncRead`, and then use `futures::Stream`
/// to access the deserialized values.
///
/// Note that the sender *must* prefix each serialized item with its size as reported by
/// `bincode::serialized_size` encoded as a four-byte network-endian encoded. See also
/// [`serialize_into`], which does this for you.
#[derive(Debug)]
pub struct AsyncBincodeReader<R, T> {
    reader: R,
    pub(crate) buffer: Vec<u8>,
    pub(crate) size: usize,
    into: PhantomData<T>,
}

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
            buffer: Vec::new(),
            size: 0,
            reader,
            into: PhantomData,
        }
    }
}

impl<R, T> Stream for AsyncBincodeReader<R, T>
where
    for<'a> T: Deserialize<'a>,
    R: AsyncRead,
{
    type Item = T;
    type Error = bincode::Error;
    fn poll(&mut self) -> Result<Async<Option<Self::Item>>, Self::Error> {
        if self.size < 4 {
            if let FillResult::EOF = try_ready!(self.fill(5).map_err(bincode::Error::from)) {
                return Ok(Async::Ready(None));
            }
        }

        let message_size: u32 = NetworkEndian::read_u32(&self.buffer[0..4]);
        let target_buffer_size = message_size as usize + 4;

        // since self.size >= 4, we know that we can't get an clean EOF here
        try_ready!(self.fill(target_buffer_size).map_err(bincode::Error::from));

        let message = bincode::deserialize(&self.buffer[4..target_buffer_size])?;
        self.size = 0;
        Ok(Async::Ready(Some(message)))
    }
}

enum FillResult {
    Filled,
    EOF,
}

impl<R, T> AsyncBincodeReader<R, T>
where
    for<'a> T: Deserialize<'a>,
    R: tokio::io::AsyncRead,
{
    fn fill(&mut self, target_size: usize) -> Result<Async<FillResult>, tokio::io::Error> {
        if self.buffer.len() < target_size {
            self.buffer.resize(target_size, 0u8);
        }

        while self.size < target_size {
            let n = try_ready!(
                self.reader
                    .poll_read(&mut self.buffer[self.size..target_size])
            );
            if n == 0 {
                if self.size == 0 {
                    return Ok(Async::Ready(FillResult::EOF));
                } else {
                    return Err(tokio::io::Error::from(io::ErrorKind::BrokenPipe));
                }
            }
            self.size += n;
        }
        Ok(Async::Ready(FillResult::Filled))
    }
}
