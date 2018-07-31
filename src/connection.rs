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
		*self = match mem::replace(self, Connection::Killed) {
			Connection::Connecter(connecter) => connecter.poll(executor).into(),
			Connection::Connectee(connectee) => connectee.poll(executor).into(),
			Connection::ConnecterLocalClosed(connected_local_closed) => {
				connected_local_closed.poll(executor).into()
			}
			Connection::ConnecteeLocalClosed(connectee_local_closed) => {
				connectee_local_closed.poll(executor).into()
			}
			Connection::Connected(connected) => connected.poll(executor).into(),
			Connection::RemoteClosed(remote_closed) => remote_closed.poll(executor).into(),
			Connection::LocalClosed(local_closed) => local_closed.poll(executor).into(),
			Connection::Closing(closing) => closing.poll(executor).into(),
			Connection::Closed => Connection::Closed,
			Connection::Killed => Connection::Killed,
		};
	}
	#[inline(always)]
	pub fn connecting(&self) -> bool {
		match self {
			Connection::Connecter(_)
			| Connection::Connectee(_)
			| Connection::ConnecterLocalClosed(_)
			| Connection::ConnecteeLocalClosed(_) => true,
			_ => false,
		}
	}
	#[inline(always)]
	pub fn recvable(&self) -> bool {
		match self {
			Connection::Connected(_) | Connection::LocalClosed(_) => true,
			_ => false,
		}
	}
	#[inline(always)]
	pub fn recv_avail(&self) -> Option<usize> {
		if self.recvable() {
			Some(match self {
				Connection::Connected(ref connected) => connected.recv_avail(),
				Connection::LocalClosed(ref local_closed) => local_closed.recv_avail(),
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
					Connection::Connected(ref mut connected) => connected.recv(executor).unwrap()(),
					Connection::LocalClosed(ref mut local_closed) => {
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
			Connection::Connected(_) | Connection::RemoteClosed(_) => true,
			_ => false,
		}
	}
	#[inline(always)]
	pub fn send_avail(&self) -> Option<usize> {
		if self.sendable() {
			Some({
				match self {
					Connection::Connected(ref connected) => connected.send_avail(),
					Connection::RemoteClosed(ref remote_closed) => remote_closed.send_avail(),
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
					Connection::Connected(ref mut connected) => {
						connected.send(executor).unwrap()(x)
					}
					Connection::RemoteClosed(ref mut remote_closed) => {
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
			Connection::Closed => true,
			_ => false,
		}
	}
	#[inline(always)]
	pub fn valid(&self) -> bool {
		match self {
			Connection::Connecter(_)
			| Connection::Connectee(_)
			| Connection::ConnecterLocalClosed(_)
			| Connection::ConnecteeLocalClosed(_)
			| Connection::Connected(_)
			| Connection::RemoteClosed(_)
			| Connection::LocalClosed(_)
			| Connection::Closing(_)
			| Connection::Closed => true,
			Connection::Killed => false,
		}
	}
	#[inline(always)]
	pub fn closable(&self) -> bool {
		match self {
			Connection::Connecter(_)
			| Connection::Connectee(_)
			| Connection::Connected(_)
			| Connection::RemoteClosed(_) => true,
			Connection::ConnecterLocalClosed(_)
			| Connection::ConnecteeLocalClosed(_)
			| Connection::LocalClosed(_)
			| Connection::Closing(_)
			| Connection::Closed
			| Connection::Killed => false,
		}
	}
	#[must_use]
	pub fn close<'a>(&'a mut self, executor: &'a impl Notifier) -> Option<impl FnOnce() + 'a> {
		if self.closable() {
			Some(move || {
				*self = match mem::replace(self, Connection::Killed) {
					Connection::Connecter(connecter) => connecter.close(executor).into(),
					Connection::Connectee(connectee) => connectee.close(executor).into(),
					Connection::Connected(connected) => connected.close(executor).into(),
					Connection::RemoteClosed(remote_closed) => remote_closed.close(executor).into(),
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
			Connection::Connecter(_)
			| Connection::Connectee(_)
			| Connection::ConnecterLocalClosed(_)
			| Connection::ConnecteeLocalClosed(_)
			| Connection::Connected(_)
			| Connection::RemoteClosed(_)
			| Connection::LocalClosed(_)
			| Connection::Closing(_) => true,
			Connection::Closed | Connection::Killed => false,
		}
	}
	#[must_use]
	pub fn kill<'a>(&'a mut self, executor: &'a impl Notifier) -> Option<impl FnOnce() + 'a> {
		if self.killable() {
			Some(move || {
				match mem::replace(self, Connection::Killed) {
					Connection::Connecter(connecter) => connecter.kill(executor),
					Connection::Connectee(connectee) => connectee.kill(executor),
					Connection::Connected(connected) => connected.kill(executor),
					Connection::RemoteClosed(remote_closed) => remote_closed.kill(executor),
					Connection::LocalClosed(local_closed) => local_closed.kill(executor),
					Connection::ConnecterLocalClosed(connecter_local_closed) => {
						connecter_local_closed.kill(executor)
					}
					Connection::ConnecteeLocalClosed(connectee_local_closed) => {
						connectee_local_closed.kill(executor)
					}
					Connection::Closing(closing) => closing.kill(executor),
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
		Connection::Connecter(connecter)
	}
}
impl From<ConnecterPoll> for Connection {
	#[inline(always)]
	fn from(connecter_poll: ConnecterPoll) -> Self {
		match connecter_poll {
			ConnecterPoll::Connecter(connecter) => Connection::Connecter(connecter),
			ConnecterPoll::Connected(connected) => Connection::Connected(connected),
			ConnecterPoll::RemoteClosed(remote_closed) => Connection::RemoteClosed(remote_closed),
			ConnecterPoll::Killed => Connection::Killed,
		}
	}
}
impl From<Connectee> for Connection {
	#[inline(always)]
	fn from(connectee: Connectee) -> Self {
		Connection::Connectee(connectee)
	}
}
impl From<ConnecteePoll> for Connection {
	#[inline(always)]
	fn from(connectee_poll: ConnecteePoll) -> Self {
		match connectee_poll {
			ConnecteePoll::Connectee(connectee) => Connection::Connectee(connectee),
			ConnecteePoll::Connected(connected) => Connection::Connected(connected),
			ConnecteePoll::RemoteClosed(remote_closed) => Connection::RemoteClosed(remote_closed),
			ConnecteePoll::Killed => Connection::Killed,
		}
	}
}
impl From<ConnecterLocalClosed> for Connection {
	#[inline(always)]
	fn from(connecter_local_closed: ConnecterLocalClosed) -> Self {
		Connection::ConnecterLocalClosed(connecter_local_closed)
	}
}
impl From<ConnecterLocalClosedPoll> for Connection {
	#[inline(always)]
	fn from(connecter_local_closed_poll: ConnecterLocalClosedPoll) -> Self {
		match connecter_local_closed_poll {
			ConnecterLocalClosedPoll::ConnecterLocalClosed(connecter_local_closed) => {
				Connection::ConnecterLocalClosed(connecter_local_closed)
			}
			ConnecterLocalClosedPoll::LocalClosed(local_closed) => {
				Connection::LocalClosed(local_closed)
			}
			ConnecterLocalClosedPoll::Closing(closing) => Connection::Closing(closing),
			ConnecterLocalClosedPoll::Closed => Connection::Closed,
			ConnecterLocalClosedPoll::Killed => Connection::Killed,
		}
	}
}
impl From<ConnecteeLocalClosed> for Connection {
	#[inline(always)]
	fn from(connectee_local_closed: ConnecteeLocalClosed) -> Self {
		Connection::ConnecteeLocalClosed(connectee_local_closed)
	}
}
impl From<ConnecteeLocalClosedPoll> for Connection {
	#[inline(always)]
	fn from(connectee_local_closed_poll: ConnecteeLocalClosedPoll) -> Self {
		match connectee_local_closed_poll {
			ConnecteeLocalClosedPoll::ConnecteeLocalClosed(connectee_local_closed) => {
				Connection::ConnecteeLocalClosed(connectee_local_closed)
			}
			ConnecteeLocalClosedPoll::LocalClosed(local_closed) => {
				Connection::LocalClosed(local_closed)
			}
			ConnecteeLocalClosedPoll::Closing(closing) => Connection::Closing(closing),
			ConnecteeLocalClosedPoll::Closed => Connection::Closed,
			ConnecteeLocalClosedPoll::Killed => Connection::Killed,
		}
	}
}
impl From<Connected> for Connection {
	#[inline(always)]
	fn from(connected: Connected) -> Self {
		Connection::Connected(connected)
	}
}
impl From<ConnectedPoll> for Connection {
	#[inline(always)]
	fn from(connected_poll: ConnectedPoll) -> Self {
		match connected_poll {
			ConnectedPoll::Connected(connected) => Connection::Connected(connected),
			ConnectedPoll::RemoteClosed(remote_closed) => Connection::RemoteClosed(remote_closed),
			ConnectedPoll::Killed => Connection::Killed,
		}
	}
}
impl From<RemoteClosed> for Connection {
	#[inline(always)]
	fn from(remote_closed: RemoteClosed) -> Self {
		Connection::RemoteClosed(remote_closed)
	}
}
impl From<RemoteClosedPoll> for Connection {
	#[inline(always)]
	fn from(remote_closed_poll: RemoteClosedPoll) -> Self {
		match remote_closed_poll {
			RemoteClosedPoll::RemoteClosed(remote_closed) => {
				Connection::RemoteClosed(remote_closed)
			}
			RemoteClosedPoll::Killed => Connection::Killed,
		}
	}
}
impl From<LocalClosed> for Connection {
	#[inline(always)]
	fn from(local_closed: LocalClosed) -> Self {
		Connection::LocalClosed(local_closed)
	}
}
impl From<LocalClosedPoll> for Connection {
	#[inline(always)]
	fn from(local_closed_poll: LocalClosedPoll) -> Self {
		match local_closed_poll {
			LocalClosedPoll::LocalClosed(local_closed) => Connection::LocalClosed(local_closed),
			LocalClosedPoll::Closing(closing) => Connection::Closing(closing),
			LocalClosedPoll::Closed => Connection::Closed,
			LocalClosedPoll::Killed => Connection::Killed,
		}
	}
}
impl From<Closing> for Connection {
	#[inline(always)]
	fn from(closing: Closing) -> Self {
		Connection::Closing(closing)
	}
}
impl From<ClosingPoll> for Connection {
	#[inline(always)]
	fn from(closing_poll: ClosingPoll) -> Self {
		match closing_poll {
			ClosingPoll::Closing(closing) => Connection::Closing(closing),
			ClosingPoll::Closed => Connection::Closed,
			ClosingPoll::Killed => Connection::Killed,
		}
	}
}
