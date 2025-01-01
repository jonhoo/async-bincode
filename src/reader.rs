use bincode::Options;
use byteorder::{ByteOrder, NetworkEndian};
use bytes::buf::Buf;
use bytes::BytesMut;
use futures_core::ready;
use serde::Deserialize;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

macro_rules! make_reader {
    ($read_trait:path, $internal_poll_reader:path) => {
        /// A wrapper around an asynchronous reader that produces an asynchronous stream of
        /// bincode-decoded values.
        ///
        /// To use, provide a reader that implements
        #[doc=concat!("[`", stringify!($read_trait), "`],")]
        /// and then use [`futures_core::Stream`] to access the deserialized values.
        ///
        /// Note that the sender *must* prefix each serialized item with its size as reported by
        /// [`bincode::serialized_size`] encoded as a four-byte network-endian encoded. Use the
        /// marker trait [`AsyncDestination`] to add it automatically when using
        /// [`AsyncBincodeWriter`].
        #[derive(Debug)]
        pub struct AsyncBincodeReader<R, T>(crate::reader::AsyncBincodeReader<R, T>);

        impl<R, T> Unpin for AsyncBincodeReader<R, T> where R: Unpin {}

        impl<R, T> Default for AsyncBincodeReader<R, T>
        where
            R: Default,
        {
            fn default() -> Self {
                Self::from(R::default())
            }
        }

        impl<R, T> From<R> for AsyncBincodeReader<R, T> {
            fn from(reader: R) -> Self {
                Self(crate::reader::AsyncBincodeReader {
                    buffer: ::bytes::BytesMut::with_capacity(8192),
                    reader,
                    into: ::std::marker::PhantomData,
                })
            }
        }

        impl<R, T> AsyncBincodeReader<R, T> {
            /// Gets a reference to the underlying reader.
            ///
            /// It is inadvisable to directly read from the underlying reader.
            pub fn get_ref(&self) -> &R {
                &self.0.reader
            }

            /// Gets a mutable reference to the underlying reader.
            ///
            /// It is inadvisable to directly read from the underlying reader.
            pub fn get_mut(&mut self) -> &mut R {
                &mut self.0.reader
            }

            /// Returns a reference to the internally buffered data.
            ///
            /// This will not attempt to fill the buffer if it is empty.
            pub fn buffer(&self) -> &[u8] {
                &self.0.buffer[..]
            }

            /// Unwraps this `AsyncBincodeReader`, returning the underlying reader.
            ///
            /// Note that any leftover data in the internal buffer is lost.
            pub fn into_inner(self) -> R {
                self.0.reader
            }
        }

        impl<R, T> ::futures_core::Stream for AsyncBincodeReader<R, T>
        where
            for<'a> T: ::serde::Deserialize<'a>,
            R: $read_trait + Unpin,
        {
            type Item = Result<T, bincode::Error>;
            fn poll_next(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context,
            ) -> std::task::Poll<Option<Self::Item>> {
                std::pin::Pin::new(&mut self.0).internal_poll_next(cx, $internal_poll_reader)
            }
        }
    };
}

#[derive(Debug)]
pub(crate) struct AsyncBincodeReader<R, T> {
    pub(crate) reader: R,
    pub(crate) buffer: BytesMut,
    pub(crate) into: PhantomData<T>,
}

impl<R, T> Unpin for AsyncBincodeReader<R, T> where R: Unpin {}

enum FillResult {
    Filled,
    EOF,
}

impl<R: Unpin, T> AsyncBincodeReader<R, T>
where
    for<'a> T: Deserialize<'a>,
{
    pub(crate) fn internal_poll_next<F>(
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
            match poll_reader(Pin::new(&mut self.reader), cx, &mut rest[..]) {
                Poll::Ready(result) => {
                    match result {
                        Ok(n) => {
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
                        Err(err) => {
                            // reading failed, put the buffer back
                            rest.truncate(0);
                            self.buffer.unsplit(rest);
                            return Poll::Ready(Err(err));
                        }
                    }
                }
                Poll::Pending => {
                    // reading in progress, put the buffer back
                    rest.truncate(0);
                    self.buffer.unsplit(rest);
                    return Poll::Pending;
                }
            }
        }

        Poll::Ready(Ok(FillResult::Filled))
    }
}
