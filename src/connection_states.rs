use super::*;
use circular_buffer::CircularBuffer;
use log::trace;
#[cfg(unix)]
use nix::{errno, fcntl, libc, sys::socket, unistd};
use std::{mem, net, time};

pub struct Listener {
	fd: Fd,
	is_socket_forwarder: bool,
}
impl Listener {
	pub fn new_ephemeral(host: &net::IpAddr, executor: &impl Notifier) -> (Self, u16) {
		let process_listener = palaver::socket::socket(
			socket::AddressFamily::Inet,
			socket::SockType::Stream,
			palaver::socket::SockFlag::SOCK_NONBLOCK,
			socket::SockProtocol::Tcp,
		)
		.unwrap();
		socket::setsockopt(process_listener, socket::sockopt::ReuseAddr, &true).unwrap();
		socket::bind(
			process_listener,
			&socket::SockAddr::Inet(socket::InetAddr::from_std(&net::SocketAddr::new(*host, 0))),
		)
		.unwrap();
		socket::setsockopt(process_listener, socket::sockopt::ReusePort, &true).unwrap();
		let process_id =
			if let socket::SockAddr::Inet(inet) = socket::getsockname(process_listener).unwrap() {
				inet.to_std()
			} else {
				panic!()
			}
			.port();
		executor.add_fd(process_listener);
		socket::listen(process_listener, LISTEN_BACKLOG).unwrap();
		(
			Self {
				fd: process_listener,
				is_socket_forwarder: false,
			},
			process_id,
		)
	}
	pub fn with_fd(process_listener: Fd, executor: &impl Notifier) -> Self {
		executor.add_fd(process_listener);
		socket::listen(process_listener, LISTEN_BACKLOG).unwrap();
		Self {
			fd: process_listener,
			is_socket_forwarder: false,
		}
	}
	pub fn into_fd(self) -> Fd {
		let ret = self.fd;
		mem::forget(self);
		ret
	}
	pub fn with_socket_forwardee(
		socket_forwardee: SocketForwardee, executor: &impl Notifier,
	) -> Self {
		executor.add_fd(socket_forwardee.0);
		Self {
			fd: socket_forwardee.0,
			is_socket_forwarder: true,
		}
	}
	pub fn poll<'a, F: FnMut(&Fd) -> Option<SocketForwarder>, E: Notifier>(
		&'a mut self, executor: &'a E, accept_hook: &'a mut F,
	) -> impl Iterator<Item = (net::SocketAddr, impl FnOnce(&E) -> ConnecteePoll)> + 'a {
		itertools::unfold((), move |_| {
			loop {
				let fd = if !self.is_socket_forwarder {
					palaver::socket::accept(
						self.fd,
						palaver::socket::SockFlag::SOCK_CLOEXEC
							| palaver::socket::SockFlag::SOCK_NONBLOCK,
					)
				} else {
					SocketForwardee(self.fd).recv().and_then(|fd| {
						match palaver::socket::accept(
							fd,
							palaver::socket::SockFlag::SOCK_CLOEXEC
								| palaver::socket::SockFlag::SOCK_NONBLOCK,
						) {
							// alternative but doesn't work on mac: socket::getsockopt(fd, socket::sockopt::AcceptConn).unwrap()
							Err(nix::Error::Sys(errno::Errno::EINVAL)) => Ok(fd),
							x => {
								trace!("Listener received forwarded listener");
								assert!(self.is_socket_forwarder);
								executor.remove_fd(self.fd);
								unistd::close(self.fd).unwrap();
								assert!(
									fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFL).unwrap()
										& fcntl::OFlag::O_NONBLOCK.bits() != 0
								);
								executor.add_fd(fd);
								self.fd = fd;
								self.is_socket_forwarder = false;
								x
							}
						}
					})
				};
				match fd {
					Ok(fd) => {
						match accept_hook(&fd) {
							None => {
								if let (Ok(remote), 0) = (
									socket::getpeername(fd),
									socket::getsockopt(fd, socket::sockopt::SocketError).unwrap(),
								) {
									let remote = if let socket::SockAddr::Inet(inet) = remote {
										inet.to_std()
									} else {
										panic!()
									};
									socket::setsockopt(fd, socket::sockopt::ReusePort, &true)
										.unwrap();
									socket::setsockopt(fd, socket::sockopt::ReuseAddr, &true)
										.unwrap();
									socket::setsockopt(
										fd,
										socket::sockopt::Linger,
										&libc::linger {
											l_onoff: 1,
											l_linger: 10,
										},
									)
									.unwrap(); // assert that close is quick?? https://www.nybek.com/blog/2015/04/29/so_linger-on-non-blocking-sockets/
									socket::setsockopt(fd, socket::sockopt::TcpNoDelay, &true)
										.unwrap();
									trace!("Listener accepted {}", format_remote(remote));
									return Some((
										remote,
										(move |executor: &E| {
											let connectee = Connectee::new(fd, executor, remote);
											match &connectee {
												ConnecteePoll::Connectee(Connectee {
													fd, ..
												})
												| ConnecteePoll::Connected(Connected {
													fd, ..
												})
												| ConnecteePoll::RemoteClosed(RemoteClosed {
													fd,
													..
												}) => {
													executor.queue();
													executor.add_fd(*fd);
												}
												ConnecteePoll::Killed => (),
											}
											connectee
										}),
									));
								} else {
									unistd::close(fd).unwrap();
									trace!("Listener !accepted");
								}
							}
							Some(to) => {
								to.send(fd, false).unwrap();
							}
						}
					}
					Err(nix::Error::Sys(errno::Errno::EAGAIN)) => return None,
					Err(err) => panic!("Listener err {:?} {:?}", self.is_socket_forwarder, err,),
				}
			}
		})
	}
	pub fn close(self, executor: &impl Notifier) {
		executor.remove_fd(self.fd);
		unistd::close(self.fd).unwrap();
		mem::forget(self);
	}
}
impl Drop for Listener {
	fn drop(&mut self) {
		panic!("Don't drop Listener");
	}
}
impl fmt::Debug for Listener {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt.debug_struct("Listener")
			.field("fd", &self.fd)
			.field("socket", &socketstat::socketstat(self.fd))
			.field("is_socket_forwarder", &self.is_socket_forwarder)
			.finish()
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum ConnecterPoll {
	Connecter(Connecter),
	Connected(Connected),
	RemoteClosed(RemoteClosed),
	Killed,
}
pub struct Connecter {
	state: Option<Fd>,
	local: net::SocketAddr,
	remote: net::SocketAddr,
}
impl Connecter {
	pub fn new(
		local: net::SocketAddr, remote: net::SocketAddr, executor: &impl Notifier,
	) -> ConnecterPoll {
		trace!("Connecter connect {}", format_remote(remote));
		Self {
			state: None,
			local,
			remote,
		}
		.poll(executor)
	}
	pub fn poll(mut self, executor: &impl Notifier) -> ConnecterPoll {
		let mut count = 0;
		loop {
			count += 1;
			assert!(count < 1_000);
			match self.state {
				None => {
					let fd = palaver::socket::socket(
						socket::AddressFamily::Inet,
						socket::SockType::Stream,
						palaver::socket::SockFlag::SOCK_CLOEXEC
							| palaver::socket::SockFlag::SOCK_NONBLOCK,
						socket::SockProtocol::Tcp,
					)
					.unwrap();
					socket::setsockopt(fd, socket::sockopt::ReusePort, &true).unwrap();
					socket::setsockopt(fd, socket::sockopt::ReuseAddr, &true).unwrap();
					socket::setsockopt(
						fd,
						socket::sockopt::Linger,
						&libc::linger {
							l_onoff: 1,
							l_linger: 10,
						},
					)
					.unwrap();
					socket::setsockopt(fd, socket::sockopt::TcpNoDelay, &true).unwrap();
					socket::bind(
						fd,
						&socket::SockAddr::Inet(socket::InetAddr::from_std(&self.local)),
					)
					.unwrap();
					executor.add_fd(fd);
					trace!("Connecter connecting {}", format_remote(self.remote));
					if match socket::connect(
						fd,
						&socket::SockAddr::Inet(socket::InetAddr::from_std(&self.remote)),
					) {
						Err(nix::Error::Sys(errno::Errno::EINPROGRESS)) => true,
						Err(nix::Error::Sys(errno::Errno::EADDRNOTAVAIL)) => false,
						Err(nix::Error::Sys(errno::Errno::ECONNABORTED)) => {
							trace!("Connecter ECONNABORTED");
							false
						}
						err => panic!("Connecter err {:?}", err),
					} && socket::getsockopt(fd, socket::sockopt::SocketError).unwrap() == 0
					{
						// sometimes ECONNRESET; sometimes ECONNREFUSED (after remote segfaulted?)
						trace!(
							"Connecter connect in progress {}",
							format_remote(self.remote)
						);
						self.state = Some(fd);
					} else {
						executor.remove_fd(fd);
						unistd::close(fd).unwrap();
						let timeout = time::Instant::now() + time::Duration::new(0, 1_000_000);
						trace!(
							"Connecter reconnect {} {:?}",
							format_remote(self.remote),
							timeout
						);
						let _ = executor.add_instant(timeout);
						return ConnecterPoll::Connecter(self);
					}
				}
				Some(fd) => {
					let x = socket::getsockopt(fd, socket::sockopt::SocketError).unwrap();
					if x == 0 {
						if palaver::socket::is_connected(fd) {
							trace!("Connecter connected {}", format_remote(self.remote));
							let ret = match Connected::new(fd, executor, self.remote) {
								ConnectedPoll::Connected(x) => ConnecterPoll::Connected(x),
								ConnectedPoll::RemoteClosed(x) => ConnecterPoll::RemoteClosed(x),
								ConnectedPoll::Killed => ConnecterPoll::Killed,
							};
							mem::forget(self);
							return ret;
						} else {
							assert_ne!(self.state, None);
							return ConnecterPoll::Connecter(self);
						}
					} else {
						trace!(
							"Connecter err {} {:?}",
							format_remote(self.remote),
							errno::Errno::from_i32(x)
						);
						executor.remove_fd(fd);
						unistd::close(fd).unwrap();
						self.state = None;
					}
				}
			}
		}
	}
	pub fn close(self, executor: &impl Notifier) -> ConnecterLocalClosedPoll {
		let ret = ConnecterLocalClosed::new(self.state, self.local, self.remote, executor);
		mem::forget(self);
		ret
	}
	pub fn kill(self, executor: &impl Notifier) {
		if let Some(fd) = self.state {
			executor.remove_fd(fd);
			unistd::close(fd).unwrap();
		}
		mem::forget(self);
	}
}
impl Drop for Connecter {
	fn drop(&mut self) {
		panic!("Don't drop Connecter");
	}
}
impl fmt::Debug for Connecter {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt.debug_struct("Connecter")
			.field("state", &self.state)
			.field("socket", &self.state.map(socketstat::socketstat))
			.field("local", &self.local)
			.field("remote", &self.remote)
			.finish()
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum ConnecteePoll {
	Connectee(Connectee),
	Connected(Connected),
	RemoteClosed(RemoteClosed),
	Killed,
}
pub struct Connectee {
	fd: Fd,
	remote: net::SocketAddr,
}
impl Connectee {
	fn new(fd: Fd, executor: &impl Notifier, remote: net::SocketAddr) -> ConnecteePoll {
		Self { fd, remote }.poll(executor)
	}
	pub fn poll(self, executor: &impl Notifier) -> ConnecteePoll {
		let x = socket::getsockopt(self.fd, socket::sockopt::SocketError).unwrap();
		if x == 0 {
			if palaver::socket::is_connected(self.fd) {
				trace!("Connectee accepted {}", format_remote(self.remote));
				let ret = match Connected::new(self.fd, executor, self.remote) {
					ConnectedPoll::Connected(x) => ConnecteePoll::Connected(x),
					ConnectedPoll::RemoteClosed(x) => ConnecteePoll::RemoteClosed(x),
					ConnectedPoll::Killed => ConnecteePoll::Killed,
				};
				mem::forget(self);
				ret
			} else {
				ConnecteePoll::Connectee(self)
			}
		} else {
			trace!(
				"Connectee err {} {:?}",
				format_remote(self.remote),
				errno::Errno::from_i32(x),
			);
			ConnecteePoll::Killed
		}
	}
	pub fn close(self, executor: &impl Notifier) -> ConnecteeLocalClosedPoll {
		let ret = ConnecteeLocalClosed::new(self.fd, executor, self.remote);
		mem::forget(self);
		ret
	}
	pub fn kill(self, executor: &impl Notifier) {
		executor.remove_fd(self.fd);
		unistd::close(self.fd).unwrap();
		mem::forget(self);
	}
}
impl Drop for Connectee {
	fn drop(&mut self) {
		panic!("Don't drop Connectee");
	}
}
impl fmt::Debug for Connectee {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt.debug_struct("Connectee")
			.field("fd", &self.fd)
			.field("socket", &socketstat::socketstat(self.fd))
			.field("remote", &self.remote)
			.finish()
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum ConnecterLocalClosedPoll {
	ConnecterLocalClosed(ConnecterLocalClosed),
	LocalClosed(LocalClosed),
	Closing(Closing),
	Closed,
	Killed,
}
pub struct ConnecterLocalClosed {
	state: Option<Fd>,
	local: net::SocketAddr,
	remote: net::SocketAddr,
}
impl ConnecterLocalClosed {
	fn new(
		state: Option<Fd>, local: net::SocketAddr, remote: net::SocketAddr,
		executor: &impl Notifier,
	) -> ConnecterLocalClosedPoll {
		Self {
			state,
			local,
			remote,
		}
		.poll(executor)
	}
	pub fn poll(mut self, executor: &impl Notifier) -> ConnecterLocalClosedPoll {
		let mut count = 0;
		loop {
			count += 1;
			assert!(count < 1_000);
			match self.state {
				None => {
					mem::forget(self);
					return ConnecterLocalClosedPoll::Closed;
				}
				Some(fd) => {
					let x = socket::getsockopt(fd, socket::sockopt::SocketError).unwrap();
					if x == 0 {
						if palaver::socket::is_connected(fd) {
							trace!(
								"ConnecterLocalClosed connected {}",
								format_remote(self.remote)
							);
							let ret = match LocalClosed::new(
								fd,
								CircularBuffer::new(BUF),
								CircularBuffer::new(BUF),
								false,
								executor,
								self.remote,
							) {
								LocalClosedPoll::LocalClosed(x) => {
									ConnecterLocalClosedPoll::LocalClosed(x)
								}
								LocalClosedPoll::Closing(x) => ConnecterLocalClosedPoll::Closing(x),
								LocalClosedPoll::Closed => ConnecterLocalClosedPoll::Closed,
								LocalClosedPoll::Killed => ConnecterLocalClosedPoll::Killed,
							};
							mem::forget(self);
							return ret;
						} else {
							assert_ne!(self.state, None);
							return ConnecterLocalClosedPoll::ConnecterLocalClosed(self);
						}
					} else {
						trace!(
							"ConnecterLocalClosed err {} {:?}",
							format_remote(self.remote),
							errno::Errno::from_i32(x)
						);
						executor.remove_fd(fd);
						unistd::close(fd).unwrap();
						self.state = None;
					}
				}
			}
		}
	}
	pub fn kill(self, executor: &impl Notifier) {
		if let Some(fd) = self.state {
			executor.remove_fd(fd);
			unistd::close(fd).unwrap();
		}
		mem::forget(self);
	}
}
impl Drop for ConnecterLocalClosed {
	fn drop(&mut self) {
		panic!("Don't drop ConnecterLocalClosed");
	}
}
impl fmt::Debug for ConnecterLocalClosed {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt.debug_struct("ConnecterLocalClosed")
			.field("state", &self.state)
			.field("socket", &self.state.map(socketstat::socketstat))
			.field("local", &self.local)
			.field("remote", &self.remote)
			.finish()
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum ConnecteeLocalClosedPoll {
	ConnecteeLocalClosed(ConnecteeLocalClosed),
	LocalClosed(LocalClosed),
	Closing(Closing),
	Closed,
	Killed,
}
pub struct ConnecteeLocalClosed {
	fd: Fd,
	remote: net::SocketAddr,
}
impl ConnecteeLocalClosed {
	fn new(fd: Fd, executor: &impl Notifier, remote: net::SocketAddr) -> ConnecteeLocalClosedPoll {
		Self { fd, remote }.poll(executor)
	}
	pub fn poll(self, executor: &impl Notifier) -> ConnecteeLocalClosedPoll {
		let x = socket::getsockopt(self.fd, socket::sockopt::SocketError).unwrap();
		if x == 0 {
			if palaver::socket::is_connected(self.fd) {
				trace!(
					"ConnecteeLocalClosed accepted {}",
					format_remote(self.remote)
				);
				let ret = match LocalClosed::new(
					self.fd,
					CircularBuffer::new(BUF),
					CircularBuffer::new(BUF),
					false,
					executor,
					self.remote,
				) {
					LocalClosedPoll::LocalClosed(x) => ConnecteeLocalClosedPoll::LocalClosed(x),
					LocalClosedPoll::Closing(x) => ConnecteeLocalClosedPoll::Closing(x),
					LocalClosedPoll::Closed => ConnecteeLocalClosedPoll::Closed,
					LocalClosedPoll::Killed => ConnecteeLocalClosedPoll::Killed,
				};
				mem::forget(self);
				ret
			} else {
				ConnecteeLocalClosedPoll::ConnecteeLocalClosed(self)
			}
		} else {
			trace!(
				"ConnecteeLocalClosed err {} {:?}",
				format_remote(self.remote),
				errno::Errno::from_i32(x),
			);
			ConnecteeLocalClosedPoll::Killed
		}
	}
	pub fn kill(self, executor: &impl Notifier) {
		executor.remove_fd(self.fd);
		unistd::close(self.fd).unwrap();
		mem::forget(self);
	}
}
impl Drop for ConnecteeLocalClosed {
	fn drop(&mut self) {
		panic!("Don't drop ConnecteeLocalClosed");
	}
}
impl fmt::Debug for ConnecteeLocalClosed {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt.debug_struct("ConnecteeLocalClosed")
			.field("fd", &self.fd)
			.field("socket", &socketstat::socketstat(self.fd))
			.field("remote", &self.remote)
			.finish()
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum ConnectedPoll {
	Connected(Connected),
	RemoteClosed(RemoteClosed),
	Killed,
}
pub struct Connected {
	fd: Fd,
	send: Option<CircularBuffer<u8>>,
	recv: Option<CircularBuffer<u8>>,
	remote_closed: bool,
	remote: net::SocketAddr,
}
impl Connected {
	fn new(fd: Fd, executor: &impl Notifier, remote: net::SocketAddr) -> ConnectedPoll {
		Self {
			fd,
			send: Some(CircularBuffer::new(BUF)),
			recv: Some(CircularBuffer::new(BUF)),
			remote_closed: false,
			remote,
		}
		.poll(executor)
	}
	pub fn poll(mut self, executor: &impl Notifier) -> ConnectedPoll {
		match self.send.as_mut().unwrap().read_to_fd(self.fd) {
			Ok(_written) => (),
			Err(err) => {
				trace!("Connected err {} {:?}", format_remote(self.remote), err,);
				self.kill(executor);
				return ConnectedPoll::Killed;
			}
		}
		if !self.remote_closed {
			match self.recv.as_mut().unwrap().write_from_fd(self.fd) {
				Ok((_read, false)) => (),
				Ok((_read, true)) => {
					trace!("Connected got closed {}", format_remote(self.remote));
					#[cfg(any(target_os = "macos", target_os = "ios"))]
					assert_ne!(sockstate::sockstate(self.fd), sockstate::TcpState::ESTABLISHED, "this is a bug in macOS; see tcp_typed/src/socket_forwarder.rs for a mitigation");
					self.remote_closed = true;
				}
				Err(err) => {
					trace!("Connected err {} {:?}", format_remote(self.remote), err,);
					self.kill(executor);
					return ConnectedPoll::Killed;
				}
			}
		}
		if !self.remote_closed || self.recv.as_mut().unwrap().read_available() > 0 {
			ConnectedPoll::Connected(self)
		} else {
			let ret = match RemoteClosed::new(
				self.fd,
				self.send.take().unwrap(),
				executor,
				self.remote,
			) {
				RemoteClosedPoll::RemoteClosed(x) => ConnectedPoll::RemoteClosed(x),
				RemoteClosedPoll::Killed => ConnectedPoll::Killed,
			};
			let _ = self.recv.take().unwrap();
			mem::forget(self);
			ret
		}
	}
	#[inline(always)]
	pub fn recv_avail(&self) -> usize {
		self.recv.as_ref().unwrap().read_available()
	}
	#[must_use]
	#[inline(always)]
	pub fn recv<'a>(&'a mut self, executor: &'a impl Notifier) -> Option<impl FnOnce() -> u8 + 'a> {
		self.recv.as_mut().unwrap().read().map(|x| {
			move || {
				let ret = x();
				executor.queue();
				ret
			}
		})
	}
	#[inline(always)]
	pub fn send_avail(&self) -> usize {
		self.send.as_ref().unwrap().write_available()
	}
	#[must_use]
	#[inline(always)]
	pub fn send<'a>(&'a mut self, executor: &'a impl Notifier) -> Option<impl FnOnce(u8) + 'a> {
		self.send.as_mut().unwrap().write().map(|x| {
			move |byte| {
				x(byte);
				executor.queue();
			}
		})
	}
	pub fn close(mut self, executor: &impl Notifier) -> LocalClosedPoll {
		// TODO: simple return type, don't poll
		let ret = LocalClosed::new(
			self.fd,
			self.send.take().unwrap(),
			self.recv.take().unwrap(),
			self.remote_closed,
			executor,
			self.remote,
		);
		mem::forget(self);
		ret
	}
	pub fn kill(mut self, executor: &impl Notifier) {
		executor.remove_fd(self.fd);
		unistd::close(self.fd).unwrap();
		let _ = self.send.take().unwrap();
		let _ = self.recv.take().unwrap();
		mem::forget(self);
	}
}
impl Drop for Connected {
	fn drop(&mut self) {
		panic!("Don't drop Connected");
	}
}
impl fmt::Debug for Connected {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt.debug_struct("Connected")
			.field("fd", &self.fd)
			.field("socket", &socketstat::socketstat(self.fd))
			.field("send", &self.send)
			.field("recv", &self.recv)
			.field("remote_closed", &self.remote_closed)
			.field("remote", &self.remote)
			.finish()
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum RemoteClosedPoll {
	RemoteClosed(RemoteClosed),
	Killed,
}
pub struct RemoteClosed {
	fd: Fd,
	send: Option<CircularBuffer<u8>>,
	remote: net::SocketAddr,
}
impl RemoteClosed {
	fn new(
		fd: Fd, send: CircularBuffer<u8>, executor: &impl Notifier, remote: net::SocketAddr,
	) -> RemoteClosedPoll {
		Self {
			fd,
			send: Some(send),
			remote,
		}
		.poll(executor)
	}
	pub fn poll(mut self, executor: &impl Notifier) -> RemoteClosedPoll {
		assert_eq!(palaver::socket::unreceived(self.fd), 0);
		match self.send.as_mut().unwrap().read_to_fd(self.fd) {
			Ok(_written) => RemoteClosedPoll::RemoteClosed(self),
			Err(err) => {
				trace!("RemoteClosed err {} {:?}", format_remote(self.remote), err,);
				self.kill(executor);
				RemoteClosedPoll::Killed
			}
		}
	}
	#[inline(always)]
	pub fn send_avail(&self) -> usize {
		self.send.as_ref().unwrap().write_available()
	}
	#[must_use]
	#[inline(always)]
	pub fn send<'a>(&'a mut self, executor: &'a impl Notifier) -> Option<impl FnOnce(u8) + 'a> {
		self.send.as_mut().unwrap().write().map(|x| {
			move |byte| {
				x(byte);
				executor.queue();
			}
		})
	}
	pub fn close(mut self, executor: &impl Notifier) -> ClosingPoll {
		// TODO: simple return type, don't poll
		let ret = Closing::new(
			self.fd,
			self.send.take().unwrap(),
			false,
			executor,
			self.remote,
		);
		mem::forget(self);
		ret
	}
	pub fn kill(mut self, executor: &impl Notifier) {
		executor.remove_fd(self.fd);
		unistd::close(self.fd).unwrap();
		let _ = self.send.take().unwrap();
		mem::forget(self);
	}
}
impl Drop for RemoteClosed {
	fn drop(&mut self) {
		panic!("Don't drop RemoteClosed");
	}
}
impl fmt::Debug for RemoteClosed {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt.debug_struct("RemoteClosed")
			.field("fd", &self.fd)
			.field("socket", &socketstat::socketstat(self.fd))
			.field("send", &self.send)
			.field("remote", &self.remote)
			.finish()
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum LocalClosedPoll {
	LocalClosed(LocalClosed),
	Closing(Closing),
	Closed,
	Killed,
}
pub struct LocalClosed {
	fd: Fd,
	send: Option<CircularBuffer<u8>>,
	recv: Option<CircularBuffer<u8>>,
	remote_closed: bool,
	local_closed_given: bool,
	remote: net::SocketAddr,
}
impl LocalClosed {
	fn new(
		fd: Fd, send: CircularBuffer<u8>, recv: CircularBuffer<u8>, remote_closed: bool,
		executor: &impl Notifier, remote: net::SocketAddr,
	) -> LocalClosedPoll {
		Self {
			fd,
			send: Some(send),
			recv: Some(recv),
			remote_closed,
			local_closed_given: false,
			remote,
		}
		.poll(executor)
	}
	pub fn poll(mut self, executor: &impl Notifier) -> LocalClosedPoll {
		if self.local_closed_given && self.remote_closed {
			let x = socket::getsockopt(self.fd, socket::sockopt::SocketError).unwrap();
			if x != 0 {
				trace!(
					"LocalClosed err {} {:?}",
					format_remote(self.remote),
					errno::Errno::from_i32(x),
				);
				self.kill(executor);
				return LocalClosedPoll::Killed;
			}
		}
		if !self.local_closed_given {
			match self.send.as_mut().unwrap().read_to_fd(self.fd) {
				Ok(_written) => (),
				Err(err) => {
					trace!("LocalClosed err {} {:?}", format_remote(self.remote), err,);
					self.kill(executor);
					return LocalClosedPoll::Killed;
				}
			}
		}
		if !self.remote_closed {
			match self.recv.as_mut().unwrap().write_from_fd(self.fd) {
				Ok((_read, false)) => (),
				Ok((_read, true)) => {
					trace!("LocalClosed got closed {}", format_remote(self.remote));
					#[cfg(any(target_os = "macos", target_os = "ios"))]
					assert_ne!(sockstate::sockstate(self.fd), sockstate::TcpState::ESTABLISHED, "this is a bug in macOS; see tcp_typed/src/socket_forwarder.rs for a mitigation");
					self.remote_closed = true;
				}
				Err(err) => {
					trace!("LocalClosed err {} {:?}", format_remote(self.remote), err,);
					self.kill(executor);
					return LocalClosedPoll::Killed;
				}
			}
		}
		if !self.local_closed_given && self.send.as_mut().unwrap().read_available() == 0 {
			match socket::shutdown(self.fd, socket::Shutdown::Write) {
				Ok(()) => self.local_closed_given = true,
				Err(err) => {
					trace!("LocalClosed err {} {:?}", format_remote(self.remote), err,);
					self.kill(executor);
					return LocalClosedPoll::Killed;
				}
			}
		}
		if !self.remote_closed || self.recv.as_mut().unwrap().read_available() > 0 {
			LocalClosedPoll::LocalClosed(self)
		} else {
			let ret = match Closing::new(
				self.fd,
				self.send.take().unwrap(),
				self.local_closed_given,
				executor,
				self.remote,
			) {
				ClosingPoll::Closing(x) => LocalClosedPoll::Closing(x),
				ClosingPoll::Closed => LocalClosedPoll::Closed,
				ClosingPoll::Killed => LocalClosedPoll::Killed,
			};
			let _ = self.recv.take().unwrap();
			mem::forget(self);
			ret
		}
	}
	#[inline(always)]
	pub fn recv_avail(&self) -> usize {
		self.recv.as_ref().unwrap().read_available()
	}
	#[must_use]
	#[inline(always)]
	pub fn recv<'a>(&'a mut self, executor: &'a impl Notifier) -> Option<impl FnOnce() -> u8 + 'a> {
		self.recv.as_mut().unwrap().read().map(|x| {
			move || {
				let ret = x();
				executor.queue();
				ret
			}
		})
	}
	pub fn kill(mut self, executor: &impl Notifier) {
		executor.remove_fd(self.fd);
		unistd::close(self.fd).unwrap();
		let _ = self.send.take().unwrap();
		let _ = self.recv.take().unwrap();
		mem::forget(self);
	}
}
impl Drop for LocalClosed {
	fn drop(&mut self) {
		panic!("Don't drop LocalClosed");
	}
}
impl fmt::Debug for LocalClosed {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt.debug_struct("LocalClosed")
			.field("fd", &self.fd)
			.field("socket", &socketstat::socketstat(self.fd))
			.field("send", &self.send)
			.field("recv", &self.recv)
			.field("remote_closed", &self.remote_closed)
			.field("local_closed_given", &self.local_closed_given)
			.field("remote", &self.remote)
			.finish()
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum ClosingPoll {
	Closing(Closing),
	Closed,
	Killed,
}
pub struct Closing {
	fd: Fd,
	send: Option<CircularBuffer<u8>>,
	local_closed_given: bool,
	remote: net::SocketAddr,
}
impl Closing {
	fn new(
		fd: Fd, send: CircularBuffer<u8>, local_closed_given: bool, executor: &impl Notifier,
		remote: net::SocketAddr,
	) -> ClosingPoll {
		Self {
			fd,
			send: Some(send),
			local_closed_given,
			remote,
		}
		.poll(executor)
	}
	pub fn poll(mut self, executor: &impl Notifier) -> ClosingPoll {
		assert_eq!(palaver::socket::unreceived(self.fd), 0);
		match self.send.as_mut().unwrap().read_to_fd(self.fd) {
			Ok(_written) => (),
			Err(err) => {
				trace!("Closing err {} {:?}", format_remote(self.remote), err);
				self.kill(executor);
				return ClosingPoll::Killed;
			}
		}
		if !self.local_closed_given && self.send.as_mut().unwrap().read_available() == 0 {
			match socket::shutdown(self.fd, socket::Shutdown::Write) {
				Ok(()) => self.local_closed_given = true,
				Err(err) => {
					trace!(
						"Closing shutdown err {} {:?}",
						format_remote(self.remote),
						err,
					);
					self.kill(executor);
					return ClosingPoll::Killed;
				}
			}
		}
		if self.local_closed_given {
			if palaver::socket::unsent(self.fd) == 0 {
				trace!("Closing close {}", format_remote(self.remote));
				executor.remove_fd(self.fd);
				unistd::close(self.fd).unwrap();
				let _ = self.send.take().unwrap();
				mem::forget(self);
				return ClosingPoll::Closed;
			} else {
				let _ =
					executor.add_instant(time::Instant::now() + time::Duration::new(0, 1_000_000));
			}
		}
		ClosingPoll::Closing(self)
	}
	pub fn kill(mut self, executor: &impl Notifier) {
		executor.remove_fd(self.fd);
		unistd::close(self.fd).unwrap();
		let _ = self.send.take().unwrap();
		mem::forget(self);
	}
}
impl Drop for Closing {
	fn drop(&mut self) {
		panic!("Don't drop Closing");
	}
}
impl fmt::Debug for Closing {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt.debug_struct("Closing")
			.field("fd", &self.fd)
			.field("socket", &socketstat::socketstat(self.fd))
			.field("send", &self.send)
			.field("local_closed_given", &self.local_closed_given)
			.field("remote", &self.remote)
			.finish()
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod sockstate {
	use nix::libc;
	use std::convert::TryInto;

	use super::Fd;

	#[derive(PartialEq, Eq, Debug)]
	#[allow(non_camel_case_types)]
	pub enum TcpState {
		CLOSED,       // 0: closed
		LISTEN,       // 1: listening for connection
		SYN_SENT,     // 2: active, have sent syn
		SYN_RECEIVED, // 3: have send and received syn
		ESTABLISHED,  // 4: established
		_CLOSE_WAIT,  // 5: rcvd fin, waiting for close
		FIN_WAIT_1,   // 6: have closed, sent fin
		CLOSING,      // 7: closed xchd FIN; await FIN ACK
		LAST_ACK,     // 8: had fin and close; await FIN ACK
		FIN_WAIT_2,   // 9: have closed, fin is acked
		TIME_WAIT,    // 10: in 2*msl quiet wait after close
		RESERVED,     // 11: pseudo state: reserved
	}
	impl TcpState {
		fn from_raw(state: u8) -> Self {
			match state {
				0 => Self::CLOSED,
				1 => Self::LISTEN,
				2 => Self::SYN_SENT,
				3 => Self::SYN_RECEIVED,
				4 => Self::ESTABLISHED,
				5 => Self::_CLOSE_WAIT,
				6 => Self::FIN_WAIT_1,
				7 => Self::CLOSING,
				8 => Self::LAST_ACK,
				9 => Self::FIN_WAIT_2,
				10 => Self::TIME_WAIT,
				11 => Self::RESERVED,
				_ => unreachable!(),
			}
		}
	}

	pub fn sockstate(fd: Fd) -> TcpState {
		let mut info: tcp_connection_info = tcp_connection_info::default();
		let mut len: libc::socklen_t = std::mem::size_of::<tcp_connection_info>()
			.try_into()
			.unwrap();
		let res = unsafe {
			libc::getsockopt(
				fd,
				libc::IPPROTO_TCP,
				TCP_CONNECTION_INFO,
				{
					let info: *mut _ = &mut info;
					info
				} as *mut _,
				&mut len,
			)
		};
		let res = nix::errno::Errno::result(res).unwrap();
		assert_eq!(res, 0);
		TcpState::from_raw(info.tcpi_state)
	}

	// https://github.com/apple/darwin-xnu/blob/a449c6a3b8014d9406c2ddbdc81795da24aa7443/bsd/netinet/tcp.h

	const TCP_CONNECTION_INFO: libc::c_int = 0x106; /* State of TCP connection */

	#[derive(Copy, Clone, Default)]
	#[repr(C)]
	struct tcp_connection_info {
		tcpi_state: u8,      /* connection state */
		tcpi_snd_wscale: u8, /* Window scale for send window */
		tcpi_rcv_wscale: u8, /* Window scale for receive window */
		__pad1: u8,
		tcpi_options: u32,      /* TCP options supported */
		tcpi_flags: u32,        /* flags */
		tcpi_rto: u32,          /* retransmit timeout in ms */
		tcpi_maxseg: u32,       /* maximum segment size supported */
		tcpi_snd_ssthresh: u32, /* slow start threshold in bytes */
		tcpi_snd_cwnd: u32,     /* send congestion window in bytes */
		tcpi_snd_wnd: u32,      /* send widnow in bytes */
		tcpi_snd_sbbytes: u32,  /* bytes in send socket buffer, including in-flight data */
		tcpi_rcv_wnd: u32,      /* receive window in bytes*/
		tcpi_rttcur: u32,       /* most recent RTT in ms */
		tcpi_srtt: u32,         /* average RTT in ms */
		tcpi_rttvar: u32,       /* RTT variance */
		tcpi_tfo: u32,
		tcpi_txpackets: u64,
		tcpi_txbytes: u64,
		tcpi_txretransmitbytes: u64,
		tcpi_rxpackets: u64,
		tcpi_rxbytes: u64,
		tcpi_rxoutoforderbytes: u64,
		tcpi_txretransmitpackets: u64,
	}
}
