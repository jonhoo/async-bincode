macro_rules! make_writer {
    ($write_trait:path, $poll_close_method:ident) => {
        pub use crate::writer::{AsyncDestination, BincodeWriterFor, SyncDestination};

        /// A wrapper around an asynchronous sink that accepts, serializes, and sends bincode-encoded
        /// values.
        ///
        /// To use, provide a reader that implements
        #[doc=concat!("[`", stringify!($write_trait), "`],")]
        /// and then use [`futures_sink::Sink`] to send values.
        ///
        /// Important: Only one element at a time is written to the output writer. It is recommended
        /// to use a `BufWriter` in front of the output to batch write operations to the underlying writer.
        ///
        /// Note that an `AsyncBincodeWriter` must be of the type [`AsyncDestination`] in order to be
        /// compatible with an [`AsyncBincodeReader`] on the remote end (recall that it requires the
        /// serialized size prefixed to the serialized data). The default is [`SyncDestination`], but these
        /// can be easily toggled between using [`AsyncBincodeWriter::for_async`].
        #[derive(Debug)]
        pub struct AsyncBincodeWriter<W, T, D> {
            pub(crate) writer: W,
            pub(crate) written: usize,
            pub(crate) buffer: Vec<u8>,
            pub(crate) from: std::marker::PhantomData<T>,
            pub(crate) dest: std::marker::PhantomData<D>,
        }

        impl<W, T, D> Unpin for AsyncBincodeWriter<W, T, D> where W: Unpin {}

        impl<W, T> Default for AsyncBincodeWriter<W, T, SyncDestination>
        where
            W: Default,
        {
            fn default() -> Self {
                Self::from(W::default())
            }
        }

        impl<W, T, D> AsyncBincodeWriter<W, T, D> {
            /// Gets a reference to the underlying writer.
            ///
            /// It is inadvisable to directly write to the underlying writer.
            pub fn get_ref(&self) -> &W {
                &self.writer
            }

            /// Gets a mutable reference to the underlying writer.
            ///
            /// It is inadvisable to directly write to the underlying writer.
            pub fn get_mut(&mut self) -> &mut W {
                &mut self.writer
            }

            /// Unwraps this `AsyncBincodeWriter`, returning the underlying writer.
            ///
            /// Note that any leftover serialized data that has not yet been sent is lost.
            pub fn into_inner(self) -> W {
                self.writer
            }
        }

        impl<W, T> From<W> for AsyncBincodeWriter<W, T, SyncDestination> {
            fn from(writer: W) -> Self {
                Self {
                    buffer: Vec::new(),
                    writer,
                    written: 0,
                    from: std::marker::PhantomData,
                    dest: std::marker::PhantomData,
                }
            }
        }

        impl<W, T> AsyncBincodeWriter<W, T, SyncDestination> {
            /// Make this writer include the serialized data's size before each serialized value.
            ///
            /// This is necessary for compatibility with [`AsyncBincodeReader`].
            pub fn for_async(self) -> AsyncBincodeWriter<W, T, AsyncDestination> {
                self.make_for()
            }
        }

        impl<W, T> AsyncBincodeWriter<W, T, AsyncDestination> {
            /// Make this writer only send bincode-encoded values.
            ///
            /// This is necessary for compatibility with stock `bincode` receivers.
            pub fn for_sync(self) -> AsyncBincodeWriter<W, T, SyncDestination> {
                self.make_for()
            }
        }

        impl<W, T, D> AsyncBincodeWriter<W, T, D> {
            pub(crate) fn make_for<D2>(self) -> AsyncBincodeWriter<W, T, D2> {
                AsyncBincodeWriter {
                    buffer: self.buffer,
                    writer: self.writer,
                    written: self.written,
                    from: self.from,
                    dest: std::marker::PhantomData,
                }
            }
        }

        impl<W, T> BincodeWriterFor<T> for AsyncBincodeWriter<W, T, AsyncDestination>
        where
            T: serde::Serialize,
        {
            fn append(&mut self, item: T) -> Result<(), bincode::Error> {
                use bincode::Options;
                use byteorder::{NetworkEndian, WriteBytesExt};
                let c = bincode::options()
                    .with_limit(u32::max_value() as u64)
                    .allow_trailing_bytes();
                let size = c.serialized_size(&item)? as u32;
                self.buffer.write_u32::<NetworkEndian>(size)?;
                c.serialize_into(&mut self.buffer, &item)
            }
        }

        impl<W, T> BincodeWriterFor<T> for AsyncBincodeWriter<W, T, SyncDestination>
        where
            T: serde::Serialize,
        {
            fn append(&mut self, item: T) -> Result<(), bincode::Error> {
                use bincode::Options;
                let c = bincode::options().allow_trailing_bytes();
                c.serialize_into(&mut self.buffer, &item)
            }
        }

        impl<W, T, D> futures_sink::Sink<T> for AsyncBincodeWriter<W, T, D>
        where
            T: serde::Serialize,
            W: $write_trait + Unpin,
            Self: BincodeWriterFor<T>,
        {
            type Error = bincode::Error;

            fn poll_ready(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                // allow us to borrow fields separately
                let this = self.get_mut();

                // write stuff out if we need to
                while this.written != this.buffer.len() {
                    let n = futures_core::ready!(std::pin::Pin::new(&mut this.writer)
                        .poll_write(cx, &this.buffer[this.written..]))?;
                    this.written += n;
                }

                // cleanup the buffer
                this.buffer.clear();
                this.written = 0;
                std::task::Poll::Ready(Ok(()))
            }

            fn start_send(mut self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
                // NOTE: in theory we could have a short-circuit here that tries to have bincode write
                // directly into self.writer. this would be way more efficient in the common case as we
                // don't have to do the extra buffering. the idea would be to serialize fist, and *if*
                // it errors, see how many bytes were written, serialize again into a Vec, and then
                // keep only the bytes following the number that were written in our buffer.
                // unfortunately, bincode will not tell us that number at the moment, and instead just
                // fail.

                self.append(item)?;
                Ok(())
            }

            fn poll_flush(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                std::pin::Pin::new(&mut self.writer)
                    .poll_flush(cx)
                    .map_err(bincode::Error::from)
            }

            fn poll_close(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                // `poll_ready` in `poll_close` is fine because in order to get to the first call to `poll_close`,
                // we must have already emptied the buffer, and thus the call to `poll_ready` must no longer
                // be calling `poll_write` on the underlying writer on re-entry.
                futures_core::ready!(self.as_mut().poll_ready(cx))?;

                // according to the `Sink` documentation, calling `poll_close` implies `poll_flush`, so
                // explicitly calling `poll_flush` is not needed here.
                std::pin::Pin::new(&mut self.writer)
                    .$poll_close_method(cx)
                    .map_err(bincode::Error::from)
            }
        }
    };
}

/// A marker that indicates that the wrapping type is compatible with `AsyncBincodeReader`.
#[derive(Debug)]
pub struct AsyncDestination;

/// A marker that indicates that the wrapping type is compatible with stock `bincode` receivers.
#[derive(Debug)]
pub struct SyncDestination;

#[doc(hidden)]
pub trait BincodeWriterFor<T> {
    fn append(&mut self, item: T) -> Result<(), bincode::Error>;
}
