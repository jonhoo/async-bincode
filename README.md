# async-bincode

[![Crates.io](https://img.shields.io/crates/v/async-bincode.svg)](https://crates.io/crates/async-bincode)
[![Documentation](https://docs.rs/async-bincode/badge.svg)](https://docs.rs/async-bincode/)
[![codecov](https://codecov.io/gh/jonhoo/async-bincode/graph/badge.svg?token=8MgydjsaLM)](https://codecov.io/gh/jonhoo/async-bincode)

Asynchronous access to a bincode-encoded item stream.

This crate enables you to asynchronously read from a bincode-encoded stream, or write
bincoded-encoded values. `bincode` does not support this natively, as it cannot easily [resume
from stream errors while encoding or decoding](https://github.com/TyOverby/bincode/issues/229).

`async-bincode` works around that on the receive side by buffering received bytes until a full
element's worth of data has been received, and only then calling into bincode. To make this
work, it relies on the sender to prefix each encoded element with its encoded size.

On the write side, `async-bincode` buffers the serialized values, and asynchronously sends the
resulting bytestream. **Important:** Only one element at a time is written to the output writer.
It is recommended to use a BufWriter in front of the output to batch write operations to the
underlying writer. The marker trait `AsyncDestination` can be used to automatically add the
length prefix required by an `async-bincode` receiver.

