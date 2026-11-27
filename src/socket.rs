//! The kernel's L2CAP sockets on Linux: how a controller is reached, with
//! no privilege. A `SOCK_SEQPACKET` socket on a dynamic PSM — from 4097,
//! which a process binds without `CAP_NET_BIND_SERVICE` — carries Xmip's
//! RFCOMM (`session`), each SDU one frame and handed whole; the kernel pages
//! the peer, opens the ACL link, signals the channel and segments it. Its
//! address is `system`'s, the one place the crate crosses a C interface.

use std::time::Duration;

use rustix::fd::{AsFd, OwnedFd};
use rustix::io::Errno;
use rustix::net::sockopt::Timeout;
use rustix::net::{AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType};
use transport::Waiting;
use transport::error::{Result, TransportError, classify, protocol_error};
use transport::kept::Kept;
use transport::socket::ready;

use crate::address::Address;
use crate::link::{Channel, Link, Peer};
use crate::system::{L2capAddress, device_of};

/// The most an SDU carries unless the channel's ends agree on more:
/// L2CAP's default MTU. Every RFCOMM frame Xmip writes is shorter.
const MTU: usize = 672;

/// How many channels a listener holds that no receive took yet.
const BACKLOG: i32 = 8;

/// A device reached through the kernel: the peer a Send Location connects
/// to from `controller`, or the controller a Receive Location listens on,
/// on `psm`.
pub struct Kernel {
    device: Address,
    name: String,
    psm: u16,
    controller: Option<Address>,
    kept: Kept<OwnedFd>,
}

impl Kernel {
    /// `device` on `psm`, connected to from `controller` where one is named.
    #[must_use]
    pub fn new(device: Address, psm: u16, controller: Option<Address>) -> Self {
        Self {
            name: device.to_string(),
            device,
            psm,
            controller,
            kept: Kept::new(),
        }
    }

    fn socket(flags: SocketFlags) -> Result<OwnedFd> {
        let (family, kind) = (AddressFamily::BLUETOOTH, SocketType::SEQPACKET);
        rustix::net::socket_with(family, kind, flags | SocketFlags::CLOEXEC, None)
            .map_err(|errno| failed("opening an L2CAP socket", errno))
    }
}

/// `errno` from `what`, classified as every socket's failure is.
fn failed(what: &str, errno: Errno) -> TransportError {
    classify(what, &errno.into())
}

impl Link for Kernel {
    fn name(&self) -> &str {
        &self.name
    }

    fn open(&self, timeout: Duration) -> Result<Box<dyn Channel>> {
        let socket = Self::socket(SocketFlags::empty())?;
        if let Some(controller) = self.controller {
            rustix::net::bind(&socket, &L2capAddress::new(controller, 0)).map_err(|errno| {
                failed(&format!("binding to the controller {controller}"), errno)
            })?;
        }
        // The kernel's L2CAP connect waits as long as the socket's send
        // timeout, a page no device answers included, and then says it is
        // still in progress.
        rustix::net::sockopt::set_socket_timeout(&socket, Timeout::Send, Some(timeout))
            .map_err(|errno| failed("bounding the connect", errno))?;
        let connected = rustix::net::connect(&socket, &L2capAddress::new(self.device, self.psm));
        match connected {
            Ok(()) => Ok(Box::new(Socket::settled(socket, timeout)?)),
            Err(Errno::INPROGRESS | Errno::AGAIN) => Err(TransportError::retryable(format!(
                "{} did not open PSM {:#06x} within {} ms",
                self.device,
                self.psm,
                timeout.as_millis()
            ))),
            Err(errno) => Err(failed(
                &format!("connecting to {} on PSM {:#06x}", self.device, self.psm),
                errno,
            )),
        }
    }

    fn accept(&self, timeout: Duration) -> Result<Option<(Peer, Box<dyn Channel>)>> {
        let listener = self.kept.listener()?;
        let waited = ready(&[listener.as_fd()], Some(timeout))
            .map_err(|error| classify("waiting for a peer's channel", &error))?;
        if !waited.first().copied().unwrap_or(false) {
            return Ok(None);
        }
        match rustix::net::acceptfrom_with(listener, SocketFlags::CLOEXEC) {
            Ok((socket, peer)) => {
                let peer = peer
                    .as_ref()
                    .and_then(device_of)
                    .ok_or_else(|| protocol_error("the kernel named no peer for its channel"))?;
                Ok(Some((
                    Peer::Device(peer),
                    Box::new(Socket::settled(socket, timeout)?),
                )))
            }
            // The peer gave up between the wait and the accept.
            Err(Errno::AGAIN) => Ok(None),
            Err(errno) => Err(failed("accepting a peer's channel", errno)),
        }
    }

    fn waiting(&self) -> Waiting {
        Waiting::Waits("the accept waits on the kernel's L2CAP listener for a peer's channel")
    }

    fn listening(&self) -> Result<Option<&str>> {
        self.kept.listening(|| {
            let socket = Self::socket(SocketFlags::NONBLOCK)?;
            let at = format!("{} PSM {:#06x}", self.device, self.psm);
            rustix::net::bind(&socket, &L2capAddress::new(self.device, self.psm))
                .map_err(|errno| failed(&format!("binding the listener at {at}"), errno))?;
            rustix::net::listen(&socket, BACKLOG)
                .map_err(|errno| failed(&format!("listening at {at}"), errno))?;
            Ok((socket, at))
        })
    }
}

/// One connected L2CAP channel: its sends blocking, each receive waiting
/// for the peer at most its timeout.
struct Socket {
    socket: OwnedFd,
    timeout: Duration,
}

impl Socket {
    /// `socket`, each send bounded by `timeout` as each receive is.
    fn settled(socket: OwnedFd, timeout: Duration) -> Result<Self> {
        rustix::net::sockopt::set_socket_timeout(&socket, Timeout::Send, Some(timeout))
            .map_err(|errno| failed("bounding the channel's sends", errno))?;
        Ok(Self { socket, timeout })
    }
}

impl Channel for Socket {
    fn send(&mut self, sdu: &[u8]) -> Result<()> {
        let sent = rustix::net::send(&self.socket, sdu, SendFlags::NOSIGNAL)
            .map_err(|errno| failed("sending on the channel", errno))?;
        if sent == sdu.len() {
            Ok(())
        } else {
            Err(TransportError::retryable(
                "the channel took part of a frame",
            ))
        }
    }

    fn receive(&mut self) -> Result<Option<Vec<u8>>> {
        let waited = ready(&[self.socket.as_fd()], Some(self.timeout))
            .map_err(|error| classify("waiting on the channel", &error))?;
        if !waited.first().copied().unwrap_or(false) {
            return Err(TransportError::retryable("the peer went quiet"));
        }
        let mut sdu = vec![0; MTU];
        let (read, _) = rustix::net::recv(&self.socket, &mut sdu[..], RecvFlags::DONTWAIT)
            .map_err(|errno| failed("receiving on the channel", errno))?;
        if read == 0 {
            return Ok(None);
        }
        sdu.truncate(read);
        Ok(Some(sdu))
    }

    /// The channel closes as its socket does: the kernel sends the peer
    /// the disconnection request.
    fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}
