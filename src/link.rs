//! What a Bluetooth transport reaches its peer over, at the layer its RFCOMM
//! rides on: an L2CAP channel. A [`Link`] opens one to its peer and accepts
//! one a peer opened; a [`Channel`] carries RFCOMM's frames, one frame an
//! SDU, and the dialogue on it is the same whichever link carries it
//! (`session`).
//!
//! Two links are written. Xmip's own L2CAP on an ACL link, its basic frame
//! and its signaling (`l2cap`), over a [`crate::Radio`]: the in-process radio
//! speaks it ([`crate::OnRadio`]). And the kernel's L2CAP sockets on Linux
//! (`socket`), which reach a controller with no privilege: the kernel opens
//! the ACL link and the channel, and hands each SDU whole.

use std::fmt;
use std::time::Duration;

use transport::Waiting;
use transport::error::Result;

use crate::address::Address;

/// Who opened a channel: a device, by the address the kernel names it by,
/// or a radio no device address reaches this transport over, by its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Peer {
    /// A device: its address travels on the arrival as `peer.mac`, a
    /// link-layer address (ADR-0019).
    Device(Address),
    /// The radio, `loopback` in process.
    Radio(String),
}

impl fmt::Display for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(address) => address.fmt(f),
            Self::Radio(name) => f.write_str(name),
        }
    }
}

/// One L2CAP channel to RFCOMM, each RFCOMM frame on it one SDU.
pub trait Channel: Send {
    /// Put one frame on the channel.
    ///
    /// # Errors
    /// Where the channel refused it.
    fn send(&mut self, sdu: &[u8]) -> Result<()>;

    /// The next frame, or `None` where the other end closed the channel.
    ///
    /// # Errors
    /// Nothing within the channel's timeout, retryable; a channel that could
    /// not be read, or a peer that broke L2CAP.
    fn receive(&mut self) -> Result<Option<Vec<u8>>>;

    /// Close the channel, as the end that opened it does once its RFCOMM
    /// multiplexer closed.
    ///
    /// # Errors
    /// Where the close could not be said.
    fn close(self: Box<Self>) -> Result<()>;
}

/// Where a Bluetooth transport opens channels to RFCOMM and serves them.
pub trait Link: Send + Sync {
    /// Its name in an origin URI: `loopback`, or a device address.
    fn name(&self) -> &str;

    /// A channel opened to the peer, each answer on it waited for at most
    /// `timeout`.
    ///
    /// # Errors
    /// A peer that does not answer within `timeout` or refuses the channel.
    fn open(&self, timeout: Duration) -> Result<Box<dyn Channel>>;

    /// The next channel a peer opened within `timeout`, and who opened it;
    /// `None` where none did.
    ///
    /// # Errors
    /// Where what is waited on could not be read, or a peer broke L2CAP.
    fn accept(&self, timeout: Duration) -> Result<Option<(Peer, Box<dyn Channel>)>>;

    /// How a receive on it meets a peer with nothing for it.
    fn waiting(&self) -> Waiting;

    /// Bind what [`Link::accept`] waits on and say where, as a Receive
    /// Location's transport is opened (`Configured::listening`); nothing is
    /// bound by default.
    ///
    /// # Errors
    /// Where it could not be bound.
    fn listening(&self) -> Result<Option<&str>> {
        Ok(None)
    }
}
