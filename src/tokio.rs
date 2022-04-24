//! Asynchronous access to a bincode-encoded item stream using `tokio`. See the top-level
//! documentation and the documentation for [`AsyncBincodeReader`], [`AsyncBincodeWriter`], and
//! [`AsyncBincodeStream`].

make_reader!(tokio::io::AsyncRead, internal_poll_reader);
make_writer!(tokio::io::AsyncWrite, poll_shutdown);
make_stream!(
    tokio::io::AsyncRead,
    tokio::io::AsyncWrite,
    tokio::io::ReadBuf,
    ()
);

fn internal_poll_reader<R>(
    r: std::pin::Pin<&mut R>,
    cx: &mut std::task::Context,
    rest: &mut [u8],
) -> std::task::Poll<std::io::Result<usize>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = tokio::io::ReadBuf::new(rest);
    futures_core::ready!(r.poll_read(cx, &mut buf))?;
    let n = buf.filled().len();
    std::task::Poll::Ready(Ok(n))
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
        let rbuff = self.stream.0.buffer.split();
        // Then, fish out the writer
        let writer = &mut self.stream.get_mut().0;
        // And steal the writer state so it isn't lost
        let wbuff = writer.buffer.split_off(0);
        let wsize = writer.written;
        // Now split the stream
        let (r, w) = writer.get_mut().split();
        // Then put the reader back together
        let mut reader = AsyncBincodeReader::from(r);
        reader.0.buffer = rbuff;
        // And then the writer
        let mut writer: AsyncBincodeWriter<_, _, D> = AsyncBincodeWriter::from(w).make_for();
        writer.buffer = wbuff;
        writer.written = wsize;
        // All good!
        (reader, writer)
    }
}
