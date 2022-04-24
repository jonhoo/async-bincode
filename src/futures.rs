//! Asynchronous access to a bincode-encoded item stream using `futures_io`. See the top-level
//! documentation and the documentation for [`AsyncBincodeReader`], [`AsyncBincodeWriter`], and
//! [`AsyncBincodeStream`].

make_reader!(futures_io::AsyncRead, internal_poll_reader);
make_writer!(futures_io::AsyncWrite, poll_close);
make_stream!(futures_io::AsyncRead, futures_io::AsyncWrite, [u8], usize);

fn internal_poll_reader<R>(
    r: std::pin::Pin<&mut R>,
    cx: &mut std::task::Context,
    rest: &mut [u8],
) -> std::task::Poll<std::io::Result<usize>>
where
    R: futures_io::AsyncRead + Unpin,
{
    r.poll_read(cx, rest)
}
