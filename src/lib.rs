//! Streams that arrive over a Bluetooth serial port. One data link
//! connection is one Stream: what was said between its SABM and its DISC.
//!
//! RFCOMM is the serial cable Bluetooth replaced — a barcode scanner, a
//! printer, a meter on the bench — and it rides on L2CAP, the channel layer
//! every ACL link has. A Send Location opens an L2CAP channel to RFCOMM,
//! starts the multiplexer on DLCI 0, opens the server channel's data link,
//! writes the Stream in UIH frames of N1 bytes and closes the link; a
//! Receive Location is the server that answers those and takes the Stream.
//! RFCOMM is Xmip's own on every link (`session`).
//!
//! **Acceptance is at-most-once here** ([`AT_MOST_ONCE`]): RFCOMM carries
//! data in UIH frames that nothing acknowledges above the radio's own link,
//! and the client's DISC is answered as it closes the link. Each data
//! link's Stream arrives whole.
//!
//! **What carries the L2CAP channel is a [`Link`].** A controller is reached
//! through the kernel's L2CAP sockets on Linux (`socket`), with no
//! privilege: a channel on a dynamic PSM, 4097 unless a Location says
//! another, since the kernel's own RFCOMM holds PSM 3. A Location's address
//! names the device, `bt://70:a8:d3:36:de:ad`: the peer a Send Location
//! connects to, and the controller of this machine a Receive Location
//! listens on, `00:00:00:00:00:00` for any. `bt://loopback` is the radio in
//! process ([`LoopbackRadio`]), which every test and every box without a
//! controller drives, Xmip's own L2CAP on it (`l2cap`). The origin URI names
//! the peer and the server channel: `bt://70:a8:d3:36:de:ad/3`.
//!
//! Two virtual controllers on one Linux machine stand both ends for a test
//! (`controller`, behind `test-support`): the kernel and `bluetoothd` see
//! two controllers, and only the air between them is Xmip's.
//!
//! **Built with its `declaration` feature alone** the crate is
//! [`declaration`]. It is the same wherever it is built. The wait a Location
//! takes unless it says, `TIMEOUT`, and its PSM, `PSM`, are declared there
//! with it.

#[cfg(feature = "default")]
pub mod address;
#[cfg(feature = "default")]
mod configured;
#[cfg(all(
    target_os = "linux",
    feature = "default",
    any(test, feature = "test-support")
))]
pub mod controller;
#[cfg(feature = "declaration")]
pub mod declaration;
#[cfg(feature = "default")]
pub mod l2cap;
#[cfg(feature = "default")]
pub mod link;
#[cfg(feature = "default")]
mod radio;
#[cfg(feature = "default")]
pub mod rfcomm;
#[cfg(feature = "default")]
pub mod session;
#[cfg(all(target_os = "linux", feature = "default"))]
pub mod socket;
#[cfg(all(target_os = "linux", feature = "default"))]
mod system;

#[cfg(feature = "default")]
pub use address::Address;
#[cfg(feature = "default")]
pub use link::{Channel, Link, Peer};
#[cfg(feature = "default")]
pub use radio::{LoopbackRadio, OnRadio, Radio};

#[cfg(feature = "default")]
use std::sync::Arc;
#[cfg(feature = "default")]
use std::time::Duration;

#[cfg(feature = "default")]
use declaration::TIMEOUT;
#[cfg(feature = "default")]
use net::Target;
#[cfg(feature = "default")]
use transport::ArrivalIdentity;
#[cfg(feature = "default")]
use transport::error::{Result, protocol_error};
#[cfg(feature = "default")]
use transport::held::Held;
#[cfg(feature = "default")]
use transport::loopback::{FarEnd, LOOPBACK_TIMEOUT, Loopback};
#[cfg(feature = "default")]
use transport::{Acknowledgement, Arrived, Configured, Directions, Taken, Transport};

/// Why a Stream over RFCOMM cannot be acknowledged after the receive cycle.
#[cfg(feature = "default")]
pub const AT_MOST_ONCE: &str = "RFCOMM carries data in UIH frames that nothing acknowledges \
                                above the radio's own link; the client's DISC is answered as \
                                it closes the link";

/// One end of a link, client when it sends and server when it receives.
#[cfg(feature = "default")]
#[derive(Clone)]
pub struct BluetoothTransport {
    link: Arc<dyn Link>,
    channel: u8,
    timeout: Duration,
    /// Set on a loopback: the radio holds what the server took.
    loopback: Option<Arc<LoopbackRadio>>,
}

#[cfg(feature = "default")]
impl BluetoothTransport {
    /// On `radio`, Xmip's own L2CAP on it, sending to and serving server
    /// channel `channel`.
    #[must_use]
    pub fn new(radio: Arc<dyn Radio>, channel: u8) -> Self {
        Self::on(Arc::new(OnRadio(radio)), channel)
    }

    /// On `link`, sending to and serving server channel `channel`.
    #[must_use]
    pub fn on(link: Arc<dyn Link>, channel: u8) -> Self {
        Self {
            link,
            channel,
            timeout: TIMEOUT,
            loopback: None,
        }
    }

    /// `bt://<link>/<server channel>`.
    #[must_use]
    pub fn origin(&self, channel: u8) -> String {
        format!("bt://{}/{channel}", self.link.name())
    }

    /// Write `bytes` as one data link connection on server channel `channel`.
    ///
    /// # Errors
    /// A peer that does not answer, refuses the channel or the link, or
    /// answers something else.
    pub fn send_stream(&self, channel: u8, bytes: &[u8]) -> Result<()> {
        let mut opened = self.link.open(self.timeout)?;
        session::sent(&mut *opened, channel, bytes)?;
        opened.close()
    }

    /// Serve one connection: answer the peer until it closes its channel,
    /// and hand over what its data link carried, whole, a device's address
    /// on it as `peer.mac`. `None` when nobody connected in time.
    /// Acceptance is at-most-once ([`AT_MOST_ONCE`]).
    ///
    /// # Errors
    /// Where the link could not be read or the peer broke the protocol.
    pub fn receive_one(&self) -> Result<Option<Arrived>> {
        let Some((peer, mut channel)) = self.link.accept(self.timeout)? else {
            return Ok(None);
        };
        let taken = session::served(&mut *channel)?;
        channel.close()?;
        Ok(taken.map(|(dlci, bytes)| {
            let arrived = Arrived::whole(
                format!("bt://{peer}/{}", dlci >> 1),
                bytes,
                Acknowledgement::at_most_once(AT_MOST_ONCE),
            );
            match peer {
                Peer::Device(address) => arrived.from_peer_mac(&address),
                Peer::Radio(_) => arrived,
            }
        }))
    }
}

#[cfg(feature = "default")]
impl Transport for BluetoothTransport {
    fn name(&self) -> &'static str {
        "bluetooth"
    }

    fn directions(&self) -> Directions {
        Directions::BOTH
    }

    fn arrivals(&self) -> transport::Arrivals {
        transport::Arrivals::Ordered("one line or bus, answered in the order it speaks")
    }

    fn waiting(&self) -> transport::Waiting {
        self.link.waiting()
    }

    /// Nobody connecting is not an error: an empty vector. Acceptance is
    /// at-most-once here: RFCOMM acknowledges no data ([`AT_MOST_ONCE`]).
    fn receive_up_to(&self, _most: usize) -> Result<Vec<Arrived>> {
        Ok(self.receive_one()?.into_iter().collect())
    }

    fn sending(&self) -> transport::Sending {
        transport::Sending::Gathered("a Stream is gathered, then written in UIH frames of N1 bytes")
    }

    /// `target` may name a server channel, `bt://70:a8:d3:36:de:ad/3`,
    /// overriding the transport's.
    fn send_from(&self, target: &str, outgoing: transport::Outgoing<'_>) -> Result<()> {
        let bytes: &[u8] = &outgoing.gather()?;
        let channel = match Target::under(Self::SCHEMES, target)
            .map(|named| (named.authority(), named.path()))
        {
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

#[cfg(feature = "default")]
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

#[cfg(feature = "default")]
impl Loopback for BluetoothTransport {
    fn arrival_identity(&self) -> ArrivalIdentity {
        ArrivalIdentity::Unnamed(
            "the in-process radio names no device; a controller's peer arrives with its device \
             address as peer.mac",
        )
    }

    /// The server on the link, holding the Stream the link delivered.
    fn far_end(&self) -> Result<Box<dyn FarEnd>> {
        let radio = self
            .loopback
            .as_ref()
            .ok_or_else(|| protocol_error("a controller, not a loopback radio"))?;
        let radio = Arc::clone(radio);
        let origin = format!("bt://{}/", self.link.name());
        Ok(Box::new(Held::new(self.origin(self.channel), move || {
            let (dlci, bytes) = radio
                .take()
                .ok_or_else(|| protocol_error("no data link closed"))?;
            Ok(Taken::new(format!("{origin}{}", dlci >> 1), bytes))
        })))
    }

    fn send_to(&self, address: &str, payload: &[u8]) -> Result<()> {
        Self::on(Arc::clone(&self.link), self.channel)
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
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use transport::Configured;
    use transport::payload::edge_payloads;
    use xcore::settings::Applies;
    use xcore::settings::Given;

    #[test]
    fn bluetooth_declares_its_settings_and_reads_through_them() {
        assert_eq!(
            BluetoothTransport::SETTINGS.problems(),
            Vec::<String>::new()
        );
        let given = [
            ("channel".to_string(), Given::Integer(7)),
            ("timeout".to_string(), Given::Text("250ms".to_string())),
        ];
        let built = BluetoothTransport::open("loopback", Applies::Send, &given).expect("built");
        assert_eq!(built.channel, 7);
        assert_eq!(built.timeout, Duration::from_millis(250));
        let given = [("channel".to_string(), Given::Integer(3))];
        let built = BluetoothTransport::open("loopback", Applies::Receive, &given).expect("built");
        assert_eq!(built.timeout, TIMEOUT);
        let Err(refused) = BluetoothTransport::open("loopback", Applies::Receive, &[]) else {
            panic!("channel is required");
        };
        assert!(
            refused.message.contains("\"channel\""),
            "{}",
            refused.message
        );
    }

    /// The shapes a protocol breaks on, as the System Tests list them.
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
        let mut arrived = server.receive().expect("serving");
        sending.join().expect("thread").expect("sending");
        assert_eq!(arrived.len(), 1);
        let arrived = arrived.remove(0);
        assert!(!arrived.defers(), "RFCOMM is at-most-once");
        let arrived = arrived.taken().expect("taken");
        assert_eq!(arrived.bytes, [9; 300]);
        assert_eq!(arrived.origin_uri, "bt://air/5");
        assert!(
            server.receive().expect("quiet").is_empty(),
            "nobody is not an error"
        );
    }
}
