# async-bincode

[![Crates.io](https://img.shields.io/crates/v/async-bincode.svg)](https://crates.io/crates/async-bincode)
[![Documentation](https://docs.rs/async-bincode/badge.svg)](https://docs.rs/async-bincode/)
[![Build Status](https://travis-ci.org/jonhoo/async-bincode.svg?branch=master)](https://travis-ci.org/jonhoo/async-bincode)

Asynchronous access to a bincode-encoded item stream.

This crate enables you to asynchronously read from a bincode-encoded stream. `bincode` does not
support this natively, as it cannot easily resume from stream errors while decoding
(https://github.com/TyOverby/bincode/issues/229). This crate works around that issue by
buffering received bytes until a full element's worth of data has been received, and only then
calling into bincode. To make this work, it relies on the sender to prefix each encoded element
with its encoded size. See [`serialize_into`] for a convenience method that provides this.
