//! A wrapper around platform TCP socket APIs that leverages the type system to ensure correct usage.
//!
//! **[Crates.io](https://crates.io/crates/tcp_typed) │ [Repo](https://github.com/alecmocatta/tcp_typed)**
//!
//! It's quite easy to accidentally misuse the Berkeley sockets or similar APIs, resulting in ECONNRESET/EPIPE/etc, data being lost on close, and potential hangs from non-exhaustive collection of events given edge-triggered notifications.
//!
//! This library aims to make it impossible to misuse in non-unsafe code.
//!
//! If you ever see a connection reset / ECONNRESET, EPIPE, data being lost on close, or panic, then it is a bug in this library! Please file an issue with as much info as possible.
//!
//! It's designed to be used in conjunction with an implementer of the [`Notifier`] trait – for example [`notifier`](https://github.com/alecmocatta/notifier). As long as the [`Notifier`] contract is fulfilled, then this library will collect all relevent events (connected, data in, data available to be written, remote closed, bytes acked, connection errors) upon each edge-triggered notification.
//!
//! # Note
//!
//! Currently doesn't support Windows.

#![doc(html_root_url = "https://docs.rs/tcp_typed/0.1.4")]
#![warn(
	// missing_copy_implementations,
	// missing_debug_implementations,
	// missing_docs,
	trivial_casts,
	trivial_numeric_casts,
	unused_import_braces,
	unused_qualifications,
	unused_results,
	clippy::pedantic,
)] // from https://github.com/rust-unofficial/patterns/blob/master/anti_patterns/deny-warnings.md
#![allow(
	clippy::inline_always,
	clippy::doc_markdown,
	clippy::if_not_else,
	clippy::indexing_slicing,
	clippy::new_ret_no_self,
	clippy::needless_pass_by_value
)]

mod circular_buffer;
mod connection;
mod connection_states;
mod socket_forwarder;

use std::{fmt, net, time};

#[cfg(unix)]
type Fd = std::os::unix::io::RawFd;
#[cfg(windows)]
type Fd = std::os::windows::io::RawHandle;

pub use connection::*;
pub use connection_states::*;
pub use socket_forwarder::*;

/// Implementers and users are responsible for calling `fn poll(self, &impl Notifier)` on [Connection]s or the states ([Connecter], [Connectee], [ConnecterLocalClosed], etc) as instructed by calls made to it via this trait.
pub trait Notifier {
	type InstantSlot;
	/// Poll as soon as possible; equivalent to add_instant(Instant::now()).
	fn queue(&self);
	/// Poll when we receive an edge-triggered event on this file descriptor.
	fn add_fd(&self, fd: Fd);
	/// No longer poll when we receive events on this file descriptor.
	fn remove_fd(&self, fd: Fd);
	/// Poll at this (typically future) instant.
	fn add_instant(&self, instant: time::Instant) -> Self::InstantSlot;
	/// No longer poll at this specific previously added instant.
	fn remove_instant(&self, slot: Self::InstantSlot);
}

fn format_remote(addr: net::SocketAddr) -> RemoteAddr {
	RemoteAddr(addr)
}
struct RemoteAddr(net::SocketAddr);
impl fmt::Display for RemoteAddr {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}", &self.0)
	}
}

const BUF: usize = 64 * 1024;
const LISTEN_BACKLOG: usize = 128;
