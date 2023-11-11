macro_rules! make_stream {
    ($read_trait:path, $write_trait:path, $buf_type:ty, $read_ret_type:ty) => {
        /// A wrapper around an asynchronous stream that receives and sends bincode-encoded values.
        ///
        /// To use, provide a stream that implements both
        #[doc=concat!("[`", stringify!($read_trait), "`] and [`", stringify!($write_trait), "`],")]
        /// and then use [`futures_sink::Sink`] to send values and [`futures_core::Stream`] to
        /// receive them.
        ///
        /// Note that an `AsyncBincodeStream` must be of the type [`crate::AsyncDestination`] in
        /// order to be compatible with an [`AsyncBincodeReader`] on the remote end (recall that it
        /// requires the serialized size prefixed to the serialized data). The default is
        /// [`crate::SyncDestination`], but these can be easily toggled between using
        /// [`AsyncBincodeStream::for_async`].
        #[derive(Debug)]
        pub struct AsyncBincodeStream<S, R, W, D> {
            stream: AsyncBincodeReader<InternalAsyncWriter<S, W, D>, R>,
        }

        #[doc(hidden)]
        pub struct InternalAsyncWriter<S, T, D>(AsyncBincodeWriter<S, T, D>);

        impl<S: std::fmt::Debug, T, D> std::fmt::Debug for InternalAsyncWriter<S, T, D> {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                self.get_ref().fmt(f)
            }
        }

        impl<S, R, W> Default for AsyncBincodeStream<S, R, W, SyncDestination>
        where
            S: Default + $read_trait + $write_trait + Unpin,
            for<'a> R: ::serde::Deserialize<'a>,
            W: ::serde::Serialize,
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

        impl<S, R, W> From<S> for AsyncBincodeStream<S, R, W, SyncDestination>
        where
            S: $read_trait + $write_trait + Unpin,
            for<'a> R: ::serde::Deserialize<'a>,
            W: ::serde::Serialize,
        {
            fn from(stream: S) -> Self {
                AsyncBincodeStream {
                    stream: AsyncBincodeReader::from(InternalAsyncWriter(
                        AsyncBincodeWriter::from(stream),
                    )),
                }
            }
        }

        impl<S, R, W, D> AsyncBincodeStream<S, R, W, D> {
            /// Make this stream include the serialized data's size before each serialized value.
            ///
            /// This is necessary for compatability with a remote [`AsyncBincodeReader`].
            pub fn for_async(self) -> AsyncBincodeStream<S, R, W, crate::AsyncDestination> {
                self.make_for()
            }

            /// Make this stream only send bincode-encoded values.
            ///
            /// This is necessary for compatability with stock `bincode` receivers.
            pub fn for_sync(self) -> AsyncBincodeStream<S, R, W, crate::SyncDestination> {
                self.make_for()
            }
        }

        impl<S, R, W, D> AsyncBincodeStream<S, R, W, D> {
            pub(crate) fn make_for<D2>(self) -> AsyncBincodeStream<S, R, W, D2> {
                let crate::reader::AsyncBincodeReader {
                    buffer: r_buf,
                    reader:
                        InternalAsyncWriter(AsyncBincodeWriter {
                            buffer: w_buf,
                            writer,
                            written,
                            from: _,
                            dest: _,
                        }),
                    into: _,
                } = self.stream.0;

                AsyncBincodeStream {
                    stream: AsyncBincodeReader(crate::reader::AsyncBincodeReader {
                        buffer: r_buf,
                        reader: InternalAsyncWriter(AsyncBincodeWriter {
                            buffer: w_buf,
                            writer,
                            written,
                            from: std::marker::PhantomData,
                            dest: std::marker::PhantomData,
                        }),
                        into: std::marker::PhantomData,
                    }),
                }
            }
        }

        impl<S, T, D> $read_trait for InternalAsyncWriter<S, T, D>
        where
            S: $read_trait + Unpin,
        {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context,
                buf: &mut $buf_type,
            ) -> std::task::Poll<std::io::Result<$read_ret_type>> {
                std::pin::Pin::new(self.get_mut().get_mut()).poll_read(cx, buf)
            }
        }

        impl<S, T, D> std::ops::Deref for InternalAsyncWriter<S, T, D> {
            type Target = AsyncBincodeWriter<S, T, D>;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
        impl<S, T, D> std::ops::DerefMut for InternalAsyncWriter<S, T, D> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl<S, R, W, D> futures_core::Stream for AsyncBincodeStream<S, R, W, D>
        where
            S: Unpin,
            AsyncBincodeReader<InternalAsyncWriter<S, W, D>, R>:
                futures_core::Stream<Item = Result<R, bincode::Error>>,
        {
            type Item = Result<R, bincode::Error>;
            fn poll_next(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context,
            ) -> std::task::Poll<Option<Self::Item>> {
                std::pin::Pin::new(&mut self.stream).poll_next(cx)
            }
        }

        impl<S, R, W, D> futures_sink::Sink<W> for AsyncBincodeStream<S, R, W, D>
        where
            S: Unpin,
            AsyncBincodeWriter<S, W, D>: futures_sink::Sink<W, Error = bincode::Error>,
        {
            type Error = bincode::Error;

            fn poll_ready(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                std::pin::Pin::new(&mut **self.stream.get_mut()).poll_ready(cx)
            }

            fn start_send(mut self: std::pin::Pin<&mut Self>, item: W) -> Result<(), Self::Error> {
                std::pin::Pin::new(&mut **self.stream.get_mut()).start_send(item)
            }

            fn poll_flush(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                std::pin::Pin::new(&mut **self.stream.get_mut()).poll_flush(cx)
            }

            fn poll_close(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                std::pin::Pin::new(&mut **self.stream.get_mut()).poll_close(cx)
            }
        }
    };
}
