//! Xmip's own L2CAP on a [`Radio`] as a [`Channel`], both ways: the one a
//! client opens to RFCOMM's PSM through the signaling ([`Opened`]), and the
//! one a peer opened, answered by a [`Server`] ([`Served`]).

use std::sync::Arc;
use std::time::Duration;

use transport::TransportError;
use transport::error::{Result, protocol_error};

use super::{FIRST_DYNAMIC, Frame, RFCOMM_PSM, SIGNALING, SUCCESS, Server, Signal};
use crate::link::Channel;
use crate::radio::Radio;

/// The next packet on `radio` within `timeout`, which a peer owes.
fn answered(radio: &dyn Radio, timeout: Duration) -> Result<Vec<u8>> {
    radio
        .receive(timeout)?
        .ok_or_else(|| TransportError::retryable("the peer did not answer"))
}

/// `signal` sent on `radio`, and the signal that answers it.
fn signaled(radio: &dyn Radio, signal: &Signal, timeout: Duration) -> Result<Signal> {
    radio.transmit(&Frame::new(SIGNALING, &signal.encode())?.encode())?;
    Signal::decode(&Frame::decode(&answered(radio, timeout)?)?.payload)
}

/// The channel a client opened to RFCOMM's PSM through the signaling, its
/// frames on the identifier the server gave it.
pub struct Opened {
    radio: Arc<dyn Radio>,
    remote: u16,
    timeout: Duration,
}

impl Opened {
    /// The channel to RFCOMM on `radio`, each answer waited for `timeout`.
    ///
    /// # Errors
    /// A peer that does not answer, or refuses the channel.
    pub fn open(radio: Arc<dyn Radio>, timeout: Duration) -> Result<Self> {
        let request = Signal::ConnectionRequest {
            identifier: 1,
            psm: RFCOMM_PSM,
            source: FIRST_DYNAMIC,
        };
        match signaled(&*radio, &request, timeout)? {
            Signal::ConnectionResponse {
                destination,
                result: SUCCESS,
                ..
            } => Ok(Self {
                radio,
                remote: destination,
                timeout,
            }),
            Signal::ConnectionResponse { result, .. } => Err(protocol_error(format!(
                "the peer refused the RFCOMM channel with result {result}"
            ))),
            _ => Err(protocol_error("not a connection response")),
        }
    }
}

impl Channel for Opened {
    fn send(&mut self, sdu: &[u8]) -> Result<()> {
        self.radio.transmit(&Frame::new(self.remote, sdu)?.encode())
    }

    fn receive(&mut self) -> Result<Option<Vec<u8>>> {
        let frame = Frame::decode(&answered(&*self.radio, self.timeout)?)?;
        Ok(Some(frame.payload))
    }

    fn close(self: Box<Self>) -> Result<()> {
        let request = Signal::DisconnectionRequest {
            identifier: 2,
            destination: self.remote,
            source: FIRST_DYNAMIC,
        };
        signaled(&*self.radio, &request, self.timeout).map(drop)
    }
}

/// A channel a peer opened on `radio`, answered by a [`Server`].
pub struct Served {
    radio: Arc<dyn Radio>,
    server: Server,
    timeout: Duration,
}

impl Served {
    /// The channel the next peer opens within `timeout`, its connection
    /// request answered; `None` where nothing came.
    ///
    /// # Errors
    /// A peer that spoke before it opened a channel to RFCOMM.
    pub fn accept(radio: Arc<dyn Radio>, timeout: Duration) -> Result<Option<Self>> {
        let Some(first) = radio.receive(timeout)? else {
            return Ok(None);
        };
        let mut server = Server::default();
        for answer in server.handle(&first)?.answers {
            radio.transmit(&answer)?;
        }
        if !server.open() {
            return Err(protocol_error(
                "a peer spoke before it opened a channel to RFCOMM",
            ));
        }
        Ok(Some(Self {
            radio,
            server,
            timeout,
        }))
    }
}

impl Channel for Served {
    fn send(&mut self, sdu: &[u8]) -> Result<()> {
        self.radio.transmit(&self.server.carried(sdu)?)
    }

    fn receive(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            let packet = self
                .radio
                .receive(self.timeout)?
                .ok_or_else(|| TransportError::retryable("the peer went quiet"))?;
            let handled = self.server.handle(&packet)?;
            for answer in &handled.answers {
                self.radio.transmit(answer)?;
            }
            if handled.closed {
                return Ok(None);
            }
            if handled.sdu.is_some() {
                return Ok(handled.sdu);
            }
        }
    }

    fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}
