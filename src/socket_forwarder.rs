use super::*;
#[cfg(unix)]
use nix::{sys::socket, sys::uio};
use std::os;
#[cfg(unix)]
use std::os::unix::io::IntoRawFd;

#[derive(Clone)]
pub struct SocketForwarder(Fd);
pub struct SocketForwardee(pub(crate) Fd);
pub fn socket_forwarder() -> (SocketForwarder, SocketForwardee) {
	let (send, receive) = os::unix::net::UnixDatagram::pair().unwrap();
	receive.set_nonblocking(true).unwrap();
	(
		SocketForwarder(send.into_raw_fd()),
		SocketForwardee(receive.into_raw_fd()),
	)
}
impl SocketForwarder {
	pub fn send(&self, fd: Fd) -> Result<(), nix::Error> {
		let iov = [uio::IoVec::from_slice(&[])];
		let fds = [fd];
		let cmsg = [socket::ControlMessage::ScmRights(&fds)];
		socket::sendmsg(self.0, &iov, &cmsg, socket::MsgFlags::empty(), None)
			.map(|x| assert_eq!(x, 0))
	}
}
impl SocketForwardee {
	pub fn recv(&self) -> Result<Fd, nix::Error> {
		let mut buf = [0; 8];
		let iovec = [uio::IoVec::from_mut_slice(&mut buf)];
		let mut space = socket::CmsgSpace::<[Fd; 2]>::new();
		socket::recvmsg(
			self.0,
			&iovec,
			Some(&mut space),
			socket::MsgFlags::MSG_DONTWAIT,
		).map(|msg| {
			let mut iter = msg.cmsgs();
			match (iter.next(), iter.next()) {
				(Some(socket::ControlMessage::ScmRights(fds)), None) => {
					assert_eq!(msg.bytes, 0);
					assert_eq!(fds.len(), 1);
					fds[0]
				}
				_ => panic!(),
			}
		})
	}
}
