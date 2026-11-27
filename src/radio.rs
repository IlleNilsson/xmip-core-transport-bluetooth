//! An ACL link Xmip's own L2CAP is spoken on (`l2cap`): the [`Radio`], the
//! in-process one every test and every box with no controller drives
//! ([`LoopbackRadio`]), the way can-bus drives its loopback bus, and the
//! [`Link`] over any of them ([`OnRadio`]).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use transport::Waiting;
use transport::error::Result;
use xcore::lock::Locked;

use crate::l2cap::{self, Opened, Served};
use crate::link::{Channel, Link, Peer};
use crate::session::Session;

/// Where packets go and come from: one ACL link.
pub trait Radio: Send + Sync {
    /// The radio's name, for the origin URI.
    fn name(&self) -> &str;
    /// Put a packet on the link.
    ///
    /// # Errors
    /// Where the link refused it.
    fn transmit(&self, packet: &[u8]) -> Result<()>;
    /// The next packet, or `None` when nothing arrived within `timeout`.
    ///
    /// # Errors
    /// Where the link could not be read.
    fn receive(&self, timeout: Duration) -> Result<Option<Vec<u8>>>;
}

/// The server on an in-process link: what the client transmits, Xmip's
/// L2CAP server and an RFCOMM session answer, and the answer is what the
/// client receives next.
#[derive(Default)]
pub struct LoopbackRadio {
    server: Mutex<l2cap::Server>,
    session: Mutex<Session>,
    to_client: Mutex<VecDeque<Vec<u8>>>,
    arrived: Mutex<VecDeque<(u8, Vec<u8>)>>,
}

impl LoopbackRadio {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The next Stream a data link delivered whole: its DLCI and bytes.
    #[must_use]
    pub fn take(&self) -> Option<(u8, Vec<u8>)> {
        self.arrived.locked().pop_front()
    }
}

impl Radio for LoopbackRadio {
    fn name(&self) -> &'static str {
        "loopback"
    }

    fn transmit(&self, packet: &[u8]) -> Result<()> {
        let mut server = self.server.locked();
        let handled = server.handle(packet)?;
        let mut answers = handled.answers;
        if let Some(sdu) = handled.sdu {
            let answered = self.session.locked().answer(&sdu)?;
            for frame in &answered.answers {
                answers.push(server.carried(frame)?);
            }
            self.arrived.locked().extend(answered.complete);
        }
        if handled.closed {
            *self.session.locked() = Session::new();
        }
        self.to_client.locked().extend(answers);
        Ok(())
    }

    fn receive(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
        Ok(self.to_client.locked().pop_front())
    }
}

/// Xmip's own L2CAP on a [`Radio`]: a channel opened through its signaling,
/// and one a peer opened served as its signaling asks.
pub struct OnRadio(pub Arc<dyn Radio>);

impl Link for OnRadio {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn open(&self, timeout: Duration) -> Result<Box<dyn Channel>> {
        Ok(Box::new(Opened::open(Arc::clone(&self.0), timeout)?))
    }

    fn accept(&self, timeout: Duration) -> Result<Option<(Peer, Box<dyn Channel>)>> {
        let served = Served::accept(Arc::clone(&self.0), timeout)?;
        Ok(served.map(|served| {
            let channel: Box<dyn Channel> = Box::new(served);
            (Peer::Radio(self.0.name().to_string()), channel)
        }))
    }

    fn waiting(&self) -> Waiting {
        Waiting::Polled("a read of the loopback radio returns at once")
    }
}
