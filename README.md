# tcp_typed

[![Crates.io](https://img.shields.io/crates/v/tcp_typed.svg?maxAge=86400)](https://crates.io/crates/tcp_typed)
[![MIT / Apache 2.0 licensed](https://img.shields.io/crates/l/tcp_typed.svg?maxAge=2592000)](#License)
[![Build Status](https://dev.azure.com/alecmocatta/tcp_typed/_apis/build/status/tests?branchName=master)](https://dev.azure.com/alecmocatta/tcp_typed/_build/latest?branchName=master)

[Docs](https://docs.rs/tcp_typed/0.1.4)

A wrapper around platform TCP socket APIs that leverages the type system to ensure correct usage.

It's quite easy to accidentally misuse the Berkeley sockets or similar APIs, resulting in ECONNRESET/EPIPE/etc, data being lost on close, and potential hangs from non-exhaustive collection of events given edge-triggered notifications.

This library aims to make it impossible to misuse in non-unsafe code.

If you ever see a connection reset / ECONNRESET, EPIPE, data being lost on close, or panic, then it is a bug in this library! Please file an issue with as much info as possible.

It's designed to be used in conjunction with an implementer of the [`Notifier`](https://docs.rs/tcp_typed/0.1.4/tcp_typed/trait.Notifier.html) trait – for example [`notifier`](https://github.com/alecmocatta/notifier). As long as the [`Notifier`](https://docs.rs/tcp_typed/0.1.4/tcp_typed/trait.Notifier.html) contract is fulfilled, then this library will collect all relevent events (connected, data in, data available to be written, remote closed, bytes acked, connection errors) upon each edge-triggered notification.

## Note

Currently doesn't support Windows.

## License
Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE.txt](LICENSE-APACHE.txt) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT.txt](LICENSE-MIT.txt) or http://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
