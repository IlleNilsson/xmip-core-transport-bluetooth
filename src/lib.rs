#![forbid(unsafe_code)]

//! Streams that arrive over a Bluetooth serial port. One data link
//! connection is one Stream: what was said between its SABM and its DISC.
//!
//! RFCOMM is the serial cable Bluetooth replaced — a barcode scanner, a
//! printer, a meter on the bench — and it rides on L2CAP, the channel layer
//! every ACL link has. A Send Location opens the L2CAP channel to PSM 3,
//! starts the multiplexer on DLCI 0, opens the server channel's data link,
//! writes the Stream in UIH frames of N1 bytes and closes the link; a
//! Receive Location is the server that answers those and takes the Stream.
//!
//! The controller is a trait: [`LoopbackRadio`] is the server on an
//! in-process link, which every test and every box without a Bluetooth
//! controller drives, the way can-bus drives its loopback bus. A deployment's
//! radio is the operating system's socket over HCI, joining here when the
//! estate exposes one. The origin URI names the radio and the server
//! channel: `bt://loopback/3`.

pub mod l2cap;
pub mod rfcomm;
pub mod session;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use transport::error::{Result, TransportError, protocol_error};
use transport::held::Held;
use transport::loopback::{FarEnd, LOOPBACK_TIMEOUT, Loopback};
use transport::{Arrived, Directions, Transport};

use l2cap::Signal;
use rfcomm::Control;
use session::Session;

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

/// The server on an in-process link: what the client transmits, the
/// session answers, and the answer is what the client receives next.
#[derive(Default)]
pub struct LoopbackRadio {
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
        self.arrived
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }
}

impl Radio for LoopbackRadio {
    fn name(&self) -> &'static str {
        "loopback"
    }

    fn transmit(&self, packet: &[u8]) -> Result<()> {
        let response = self
            .session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .handle(packet)?;
        self.to_client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(response.answers);
        if let Some(complete) = response.complete {
            self.arrived
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back(complete);
        }
        Ok(())
    }

    fn receive(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
        Ok(self
            .to_client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front())
    }
}

/// One end of an ACL link, client when it sends and server when it receives.
#[derive(Clone)]
pub struct BluetoothTransport {
    radio: Arc<dyn Radio>,
    channel: u8,
    timeout: Duration,
    /// Set on a loopback: the radio holds what the server took.
    loopback: Option<Arc<LoopbackRadio>>,
}

impl BluetoothTransport {
    /// On `radio`, sending to and serving server channel `channel`.
    #[must_use]
    pub fn new(radio: Arc<dyn Radio>, channel: u8) -> Self {
        Self {
            radio,
            channel,
            timeout: Duration::from_secs(5),
            loopback: None,
        }
    }

    /// Give up on a peer that does not answer within `timeout`.
    #[must_use]
    pub const fn timing_out_after(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// `bt://<radio>/<server channel>`.
    #[must_use]
    pub fn origin(&self, channel: u8) -> String {
        format!("bt://{}/{channel}", self.radio.name())
    }

    fn exchange(&self, packet: &[u8]) -> Result<Vec<u8>> {
        self.radio.transmit(packet)?;
        self.radio
            .receive(self.timeout)?
            .ok_or_else(|| TransportError::retryable("the peer did not answer"))
    }

    fn signal(&self, signal: &Signal) -> Result<Signal> {
        let packet = l2cap::Frame::new(l2cap::SIGNALLING, &signal.encode())?.encode();
        let answer = l2cap::Frame::decode(&self.exchange(&packet)?)?;
        Signal::decode(&answer.payload)
    }

    /// Open the L2CAP channel to RFCOMM: the server's identifier for it.
    fn connect(&self) -> Result<u16> {
        let request = Signal::ConnectionRequest {
            identifier: 1,
            psm: l2cap::RFCOMM_PSM,
            source: l2cap::FIRST_DYNAMIC,
        };
        match self.signal(&request)? {
            Signal::ConnectionResponse {
                destination,
                result: l2cap::SUCCESS,
                ..
            } => Ok(destination),
            Signal::ConnectionResponse { result, .. } => Err(protocol_error(format!(
                "the peer refused the RFCOMM channel with result {result}"
            ))),
            _ => Err(protocol_error("not a connection response")),
        }
    }

    fn command(&self, remote: u16, dlci: u8, control: Control) -> Result<()> {
        let frame = rfcomm::Frame::command(dlci, control)?;
        let packet = l2cap::Frame::new(remote, &frame.encode())?.encode();
        let answer = l2cap::Frame::decode(&self.exchange(&packet)?)?;
        match rfcomm::Frame::decode(&answer.payload)?.control {
            Control::Ua => Ok(()),
            Control::Dm => Err(protocol_error(format!("DLCI {dlci} is not open to us"))),
            other => Err(protocol_error(format!("{other:?} where UA was due"))),
        }
    }

    /// Write `bytes` as one data link connection on server channel `channel`.
    ///
    /// # Errors
    /// A peer that does not answer, refuses the channel or the link, or
    /// answers something else.
    pub fn send_stream(&self, channel: u8, bytes: &[u8]) -> Result<()> {
        let dlci = channel << 1;
        let remote = self.connect()?;
        self.command(remote, 0, Control::Sabm)?;
        self.command(remote, dlci, Control::Sabm)?;
        for chunk in bytes.chunks(rfcomm::N1) {
            let frame = rfcomm::Frame::new(dlci, Control::Uih, chunk)?;
            self.radio
                .transmit(&l2cap::Frame::new(remote, &frame.encode())?.encode())?;
        }
        self.command(remote, dlci, Control::Disc)?;
        self.command(remote, 0, Control::Disc)?;
        let request = Signal::DisconnectionRequest {
            identifier: 2,
            destination: remote,
            source: l2cap::FIRST_DYNAMIC,
        };
        self.signal(&request).map(drop)
    }

    /// Serve one connection: answer the peer until a data link closes, and
    /// hand over what it carried. `None` when nobody connected in time.
    ///
    /// # Errors
    /// Where the link could not be read or the peer broke the protocol.
    pub fn receive_one(&self) -> Result<Option<Arrived>> {
        let mut session = Session::new();
        let Some(first) = self.radio.receive(self.timeout)? else {
            return Ok(None);
        };
        let mut packet = first;
        let mut taken = None;
        // Served to the end: the peer closes the link, the multiplexer and
        // the channel after its data, and each of those is answered.
        loop {
            let response = session.handle(&packet)?;
            for answer in &response.answers {
                self.radio.transmit(answer)?;
            }
            if let Some((dlci, bytes)) = response.complete {
                taken = Some(Arrived::new(self.origin(dlci >> 1), bytes));
            }
            if response.closed {
                return Ok(taken);
            }
            packet = self
                .radio
                .receive(self.timeout)?
                .ok_or_else(|| TransportError::retryable("the peer went quiet"))?;
        }
    }
}

impl Transport for BluetoothTransport {
    fn name(&self) -> &'static str {
        "bluetooth"
    }

    fn directions(&self) -> Directions {
        Directions::BOTH
    }

    /// Nobody connecting is not an error: an empty vector.
    fn receive(&self) -> Result<Vec<Arrived>> {
        Ok(self.receive_one()?.into_iter().collect())
    }

    /// `target` may name a server channel, `bt://radio/3`, overriding the
    /// transport's.
    fn send(&self, target: &str, bytes: &[u8]) -> Result<()> {
        let channel = match transport::socket::target("bt", target) {
            Some((_, channel)) if !channel.is_empty() => channel
                .parse()
                .ok()
                .filter(|channel| (1..=30).contains(channel))
                .ok_or_else(|| protocol_error(format!("{channel:?} is not a server channel")))?,
            _ => self.channel,
        };
        self.send_stream(channel, bytes)
    }
}

impl BluetoothTransport {
    /// Both ends on one in-process link: a client on server channel 3 and
    /// the server that answers it, the loopback timeout on the client.
    #[must_use]
    pub fn loopback() -> Self {
        let radio = Arc::new(LoopbackRadio::new());
        let mut transport =
            Self::new(Arc::clone(&radio) as Arc<dyn Radio>, 3).timing_out_after(LOOPBACK_TIMEOUT);
        transport.loopback = Some(radio);
        transport
    }
}

impl Loopback for BluetoothTransport {
    /// The server on the link, holding the Stream the link delivered.
    fn far_end(&self) -> Result<Box<dyn FarEnd>> {
        let radio = self
            .loopback
            .as_ref()
            .ok_or_else(|| protocol_error("a controller, not a loopback radio"))?;
        let radio = Arc::clone(radio);
        let origin = format!("bt://{}/", self.radio.name());
        Ok(Box::new(Held::new(self.origin(self.channel), move || {
            let (dlci, bytes) = radio
                .take()
                .ok_or_else(|| protocol_error("no data link closed"))?;
            Ok(Arrived::new(format!("{origin}{}", dlci >> 1), bytes))
        })))
    }

    fn send_to(&self, address: &str, payload: &[u8]) -> Result<()> {
        Self::new(Arc::clone(&self.radio), self.channel)
            .timing_out_after(self.timeout)
            .send(address, payload)
    }

    /// In order on one thread: the server lives in the radio and answers as
    /// the client sends, so the send goes first and the take finds it.
    fn exchanges_in_order(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transport::payload::edge_payloads;

    /// The shapes a protocol breaks on, as the Playground lists them.
    fn payloads() -> Vec<(&'static str, Vec<u8>)> {
        let mut payloads = edge_payloads();
        payloads.extend([(
            "a mebibyte",
            (0..1u32 << 20)
                .map(|n| u8::try_from(n * 31 % 256).unwrap_or(0))
                .collect(),
        )]);
        payloads
    }

    #[test]
    fn a_loopback_round_carries_a_stream_as_one_data_link() {
        let loopback = BluetoothTransport::loopback();
        let arrived = loopback.round(b"scan 4006381333931").expect("round");
        assert_eq!(arrived.bytes, b"scan 4006381333931");
        assert_eq!(arrived.origin_uri, "bt://loopback/3");
        assert!(loopback.ceiling().is_none());
        assert!(loopback.refuses(b"anything").is_none());
        assert_eq!(loopback.name(), "bluetooth");
        assert!(loopback.directions().receives() && loopback.directions().sends());
        assert!(loopback.claims().is_none());
    }

    #[test]
    fn the_loopback_returns_the_edges_whole() {
        let loopback = BluetoothTransport::loopback();
        for (name, bytes) in payloads() {
            let arrived = loopback
                .round(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(arrived.bytes, bytes, "{name}");
        }
    }

    #[test]
    fn a_target_names_the_server_channel_and_a_bad_one_is_refused() {
        let loopback = BluetoothTransport::loopback();
        loopback.send("bt://loopback/7", b"seven").expect("sending");
        let radio = loopback.loopback.as_ref().expect("loopback");
        assert_eq!(radio.take(), Some((14, b"seven".to_vec())));
        assert!(loopback.send("bt://loopback/31", b"x").is_err());
        assert!(loopback.send("bt://loopback/zero", b"x").is_err());
        let error = loopback.round(b"").err();
        assert!(
            error.is_none(),
            "an empty Stream is a link with no UIH frames"
        );
    }

    #[test]
    fn the_transport_serves_a_connection_over_any_radio() {
        // Two ends of one air: what one transmits, the other receives. The
        // client runs on another thread and the server end takes the Stream.
        struct Air {
            up: Mutex<VecDeque<Vec<u8>>>,
            down: Mutex<VecDeque<Vec<u8>>>,
        }
        struct End(Arc<Air>, bool);
        impl Radio for End {
            fn name(&self) -> &'static str {
                "air"
            }
            fn transmit(&self, packet: &[u8]) -> Result<()> {
                let queue = if self.1 { &self.0.down } else { &self.0.up };
                queue.lock().expect("lock").push_back(packet.to_vec());
                Ok(())
            }
            fn receive(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
                let queue = if self.1 { &self.0.up } else { &self.0.down };
                let deadline = std::time::Instant::now() + timeout;
                loop {
                    if let Some(packet) = queue.lock().expect("lock").pop_front() {
                        return Ok(Some(packet));
                    }
                    if std::time::Instant::now() > deadline {
                        return Ok(None);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
        let air = Arc::new(Air {
            up: Mutex::new(VecDeque::new()),
            down: Mutex::new(VecDeque::new()),
        });
        let server = BluetoothTransport::new(Arc::new(End(Arc::clone(&air), true)), 5)
            .timing_out_after(Duration::from_secs(2));
        assert!(
            server.far_end().is_err(),
            "a real radio has no server inside"
        );
        let client = BluetoothTransport::new(Arc::new(End(air, false)), 5)
            .timing_out_after(Duration::from_secs(2));
        let sending = std::thread::spawn(move || client.send("bt://air/5", &[9; 300]));
        let arrived = server.receive().expect("serving");
        sending.join().expect("thread").expect("sending");
        assert_eq!(arrived.len(), 1);
        assert_eq!(arrived[0].bytes, [9; 300]);
        assert_eq!(arrived[0].origin_uri, "bt://air/5");
        assert!(
            server.receive().expect("quiet").is_empty(),
            "nobody is not an error"
        );
    }
}
