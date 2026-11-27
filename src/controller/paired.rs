//! The far end a test stands on the second of two virtual controllers: a
//! Receive Location's transport listening there through the kernel's L2CAP
//! sockets, holding the controllers while it waits for its one Stream.

use std::sync::Arc;
use std::time::Duration;

use transport::error::{Result, TransportError};
use transport::{Configured, FarEnd, Taken};

use super::Controllers;
use crate::BluetoothTransport;
use crate::address::Address;
use crate::socket::Kernel;

/// A transport listening on the second controller, and both controllers.
pub struct Paired {
    server: BluetoothTransport,
    controllers: Controllers,
    address: String,
}

impl Paired {
    /// Two controllers, and a transport on server channel `channel`
    /// listening at the second on `psm`, every wait `timeout`.
    ///
    /// # Errors
    /// As [`Controllers::pair`], or where the listener could not be bound.
    pub fn stood(psm: u16, channel: u8, timeout: Duration) -> Result<Self> {
        let controllers = Controllers::pair(timeout)?;
        let far = controllers.far();
        let server = BluetoothTransport::on(Arc::new(Kernel::new(far, psm, None)), channel)
            .timing_out_after(timeout);
        server.listening()?;
        Ok(Self {
            server,
            address: far.to_string(),
            controllers,
        })
    }

    /// The first controller: what a near end connects from.
    #[must_use]
    pub fn near(&self) -> Address {
        self.controllers.near()
    }
}

impl FarEnd for Paired {
    /// The second controller's device address.
    fn address(&self) -> &str {
        &self.address
    }

    fn take_one(self: Box<Self>) -> Result<Taken> {
        let arrived = self
            .server
            .receive_one()?
            .ok_or_else(|| TransportError::retryable("no peer opened a channel"))?;
        arrived.taken()
    }
}
