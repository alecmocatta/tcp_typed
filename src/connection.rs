use super::*;
use std::{mem, net};

/// Essentially a dynamically-typed connection, wrapping all of the individual states in an enum and providing methods that are available or not dynamically (by returning an `Option<impl FnOnce(..)>`).
#[derive(Debug)]
pub enum Connection {
	Connecter(Connecter),
	Connectee(Connectee),
	ConnecterLocalClosed(ConnecterLocalClosed),
	ConnecteeLocalClosed(ConnecteeLocalClosed),
	Connected(Connected),
	RemoteClosed(RemoteClosed),
	LocalClosed(LocalClosed),
	Closing(Closing),
	Closed,
	Killed,
}
impl Connection {
	#[must_use]
	pub fn connect(
		local: net::SocketAddr, remote: net::SocketAddr, executor: &impl Notifier,
	) -> Self {
		Connecter::new(local, remote, executor).into()
	}
	pub fn poll(&mut self, executor: &impl Notifier) {
		*self = match mem::replace(self, Self::Killed) {
			Self::Connecter(connecter) => connecter.poll(executor).into(),
			Self::Connectee(connectee) => connectee.poll(executor).into(),
			Self::ConnecterLocalClosed(connected_local_closed) => {
				connected_local_closed.poll(executor).into()
			}
			Self::ConnecteeLocalClosed(connectee_local_closed) => {
				connectee_local_closed.poll(executor).into()
			}
			Self::Connected(connected) => connected.poll(executor).into(),
			Self::RemoteClosed(remote_closed) => remote_closed.poll(executor).into(),
			Self::LocalClosed(local_closed) => local_closed.poll(executor).into(),
			Self::Closing(closing) => closing.poll(executor).into(),
			Self::Closed => Self::Closed,
			Self::Killed => Self::Killed,
		};
	}
	#[inline(always)]
	pub fn connecting(&self) -> bool {
		match self {
			Self::Connecter(_)
			| Self::Connectee(_)
			| Self::ConnecterLocalClosed(_)
			| Self::ConnecteeLocalClosed(_) => true,
			_ => false,
		}
	}
	#[inline(always)]
	pub fn recvable(&self) -> bool {
		match self {
			Self::Connected(_) | Self::LocalClosed(_) => true,
			_ => false,
		}
	}
	#[inline(always)]
	pub fn recv_avail(&self) -> Option<usize> {
		if self.recvable() {
			Some(match self {
				Self::Connected(ref connected) => connected.recv_avail(),
				Self::LocalClosed(ref local_closed) => local_closed.recv_avail(),
				_ => unreachable!(),
			})
		} else {
			None
		}
	}
	#[must_use]
	#[inline(always)]
	pub fn recv<'a>(&'a mut self, executor: &'a impl Notifier) -> Option<impl FnOnce() -> u8 + 'a> {
		self.recv_avail().and_then(move |avail| {
			if avail > 0 {
				Some(move || match self {
					Self::Connected(ref mut connected) => connected.recv(executor).unwrap()(),
					Self::LocalClosed(ref mut local_closed) => {
						local_closed.recv(executor).unwrap()()
					}
					_ => unreachable!(),
				})
			} else {
				None
			}
		})
	}
	#[inline(always)]
	pub fn sendable(&self) -> bool {
		match self {
			Self::Connected(_) | Self::RemoteClosed(_) => true,
			_ => false,
		}
	}
	#[inline(always)]
	pub fn send_avail(&self) -> Option<usize> {
		if self.sendable() {
			Some({
				match self {
					Self::Connected(ref connected) => connected.send_avail(),
					Self::RemoteClosed(ref remote_closed) => remote_closed.send_avail(),
					_ => unreachable!(),
				}
			})
		} else {
			None
		}
	}
	#[must_use]
	#[inline(always)]
	pub fn send<'a>(&'a mut self, executor: &'a impl Notifier) -> Option<impl FnOnce(u8) + 'a> {
		self.send_avail().and_then(move |avail| {
			if avail > 0 {
				Some(move |x| match self {
					Self::Connected(ref mut connected) => connected.send(executor).unwrap()(x),
					Self::RemoteClosed(ref mut remote_closed) => {
						remote_closed.send(executor).unwrap()(x)
					}
					_ => unreachable!(),
				})
			} else {
				None
			}
		})
	}
	#[inline(always)]
	pub fn closed(&self) -> bool {
		match self {
			Self::Closed => true,
			_ => false,
		}
	}
	#[inline(always)]
	pub fn valid(&self) -> bool {
		match self {
			Self::Connecter(_)
			| Self::Connectee(_)
			| Self::ConnecterLocalClosed(_)
			| Self::ConnecteeLocalClosed(_)
			| Self::Connected(_)
			| Self::RemoteClosed(_)
			| Self::LocalClosed(_)
			| Self::Closing(_)
			| Self::Closed => true,
			Self::Killed => false,
		}
	}
	#[inline(always)]
	pub fn closable(&self) -> bool {
		match self {
			Self::Connecter(_)
			| Self::Connectee(_)
			| Self::Connected(_)
			| Self::RemoteClosed(_) => true,
			Self::ConnecterLocalClosed(_)
			| Self::ConnecteeLocalClosed(_)
			| Self::LocalClosed(_)
			| Self::Closing(_)
			| Self::Closed
			| Self::Killed => false,
		}
	}
	#[must_use]
	pub fn close<'a>(&'a mut self, executor: &'a impl Notifier) -> Option<impl FnOnce() + 'a> {
		if self.closable() {
			Some(move || {
				*self = match mem::replace(self, Self::Killed) {
					Self::Connecter(connecter) => connecter.close(executor).into(),
					Self::Connectee(connectee) => connectee.close(executor).into(),
					Self::Connected(connected) => connected.close(executor).into(),
					Self::RemoteClosed(remote_closed) => remote_closed.close(executor).into(),
					_ => unreachable!(),
				};
			})
		} else {
			None
		}
	}
	#[inline(always)]
	pub fn killable(&self) -> bool {
		match self {
			Self::Connecter(_)
			| Self::Connectee(_)
			| Self::ConnecterLocalClosed(_)
			| Self::ConnecteeLocalClosed(_)
			| Self::Connected(_)
			| Self::RemoteClosed(_)
			| Self::LocalClosed(_)
			| Self::Closing(_) => true,
			Self::Closed | Self::Killed => false,
		}
	}
	#[must_use]
	pub fn kill<'a>(&'a mut self, executor: &'a impl Notifier) -> Option<impl FnOnce() + 'a> {
		if self.killable() {
			Some(move || {
				match mem::replace(self, Self::Killed) {
					Self::Connecter(connecter) => connecter.kill(executor),
					Self::Connectee(connectee) => connectee.kill(executor),
					Self::Connected(connected) => connected.kill(executor),
					Self::RemoteClosed(remote_closed) => remote_closed.kill(executor),
					Self::LocalClosed(local_closed) => local_closed.kill(executor),
					Self::ConnecterLocalClosed(connecter_local_closed) => {
						connecter_local_closed.kill(executor)
					}
					Self::ConnecteeLocalClosed(connectee_local_closed) => {
						connectee_local_closed.kill(executor)
					}
					Self::Closing(closing) => closing.kill(executor),
					_ => unreachable!(),
				};
			})
		} else {
			None
		}
	}
}
impl From<Connecter> for Connection {
	#[inline(always)]
	fn from(connecter: Connecter) -> Self {
		Self::Connecter(connecter)
	}
}
impl From<ConnecterPoll> for Connection {
	#[inline(always)]
	fn from(connecter_poll: ConnecterPoll) -> Self {
		match connecter_poll {
			ConnecterPoll::Connecter(connecter) => Self::Connecter(connecter),
			ConnecterPoll::Connected(connected) => Self::Connected(connected),
			ConnecterPoll::RemoteClosed(remote_closed) => Self::RemoteClosed(remote_closed),
			ConnecterPoll::Killed => Self::Killed,
		}
	}
}
impl From<Connectee> for Connection {
	#[inline(always)]
	fn from(connectee: Connectee) -> Self {
		Self::Connectee(connectee)
	}
}
impl From<ConnecteePoll> for Connection {
	#[inline(always)]
	fn from(connectee_poll: ConnecteePoll) -> Self {
		match connectee_poll {
			ConnecteePoll::Connectee(connectee) => Self::Connectee(connectee),
			ConnecteePoll::Connected(connected) => Self::Connected(connected),
			ConnecteePoll::RemoteClosed(remote_closed) => Self::RemoteClosed(remote_closed),
			ConnecteePoll::Killed => Self::Killed,
		}
	}
}
impl From<ConnecterLocalClosed> for Connection {
	#[inline(always)]
	fn from(connecter_local_closed: ConnecterLocalClosed) -> Self {
		Self::ConnecterLocalClosed(connecter_local_closed)
	}
}
impl From<ConnecterLocalClosedPoll> for Connection {
	#[inline(always)]
	fn from(connecter_local_closed_poll: ConnecterLocalClosedPoll) -> Self {
		match connecter_local_closed_poll {
			ConnecterLocalClosedPoll::ConnecterLocalClosed(connecter_local_closed) => {
				Self::ConnecterLocalClosed(connecter_local_closed)
			}
			ConnecterLocalClosedPoll::LocalClosed(local_closed) => Self::LocalClosed(local_closed),
			ConnecterLocalClosedPoll::Closing(closing) => Self::Closing(closing),
			ConnecterLocalClosedPoll::Closed => Self::Closed,
			ConnecterLocalClosedPoll::Killed => Self::Killed,
		}
	}
}
impl From<ConnecteeLocalClosed> for Connection {
	#[inline(always)]
	fn from(connectee_local_closed: ConnecteeLocalClosed) -> Self {
		Self::ConnecteeLocalClosed(connectee_local_closed)
	}
}
impl From<ConnecteeLocalClosedPoll> for Connection {
	#[inline(always)]
	fn from(connectee_local_closed_poll: ConnecteeLocalClosedPoll) -> Self {
		match connectee_local_closed_poll {
			ConnecteeLocalClosedPoll::ConnecteeLocalClosed(connectee_local_closed) => {
				Self::ConnecteeLocalClosed(connectee_local_closed)
			}
			ConnecteeLocalClosedPoll::LocalClosed(local_closed) => Self::LocalClosed(local_closed),
			ConnecteeLocalClosedPoll::Closing(closing) => Self::Closing(closing),
			ConnecteeLocalClosedPoll::Closed => Self::Closed,
			ConnecteeLocalClosedPoll::Killed => Self::Killed,
		}
	}
}
impl From<Connected> for Connection {
	#[inline(always)]
	fn from(connected: Connected) -> Self {
		Self::Connected(connected)
	}
}
impl From<ConnectedPoll> for Connection {
	#[inline(always)]
	fn from(connected_poll: ConnectedPoll) -> Self {
		match connected_poll {
			ConnectedPoll::Connected(connected) => Self::Connected(connected),
			ConnectedPoll::RemoteClosed(remote_closed) => Self::RemoteClosed(remote_closed),
			ConnectedPoll::Killed => Self::Killed,
		}
	}
}
impl From<RemoteClosed> for Connection {
	#[inline(always)]
	fn from(remote_closed: RemoteClosed) -> Self {
		Self::RemoteClosed(remote_closed)
	}
}
impl From<RemoteClosedPoll> for Connection {
	#[inline(always)]
	fn from(remote_closed_poll: RemoteClosedPoll) -> Self {
		match remote_closed_poll {
			RemoteClosedPoll::RemoteClosed(remote_closed) => Self::RemoteClosed(remote_closed),
			RemoteClosedPoll::Killed => Self::Killed,
		}
	}
}
impl From<LocalClosed> for Connection {
	#[inline(always)]
	fn from(local_closed: LocalClosed) -> Self {
		Self::LocalClosed(local_closed)
	}
}
impl From<LocalClosedPoll> for Connection {
	#[inline(always)]
	fn from(local_closed_poll: LocalClosedPoll) -> Self {
		match local_closed_poll {
			LocalClosedPoll::LocalClosed(local_closed) => Self::LocalClosed(local_closed),
			LocalClosedPoll::Closing(closing) => Self::Closing(closing),
			LocalClosedPoll::Closed => Self::Closed,
			LocalClosedPoll::Killed => Self::Killed,
		}
	}
}
impl From<Closing> for Connection {
	#[inline(always)]
	fn from(closing: Closing) -> Self {
		Self::Closing(closing)
	}
}
impl From<ClosingPoll> for Connection {
	#[inline(always)]
	fn from(closing_poll: ClosingPoll) -> Self {
		match closing_poll {
			ClosingPoll::Closing(closing) => Self::Closing(closing),
			ClosingPoll::Closed => Self::Closed,
			ClosingPoll::Killed => Self::Killed,
		}
	}
}
