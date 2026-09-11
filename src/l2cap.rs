//! The L2CAP basic frame — a little-endian length, a channel identifier and
//! the payload — and the signalling on channel 1 that opens and closes a
//! connection-oriented channel to a protocol's PSM. RFCOMM listens on PSM 3.

use transport::error::{Result, protocol_error};

/// The signalling channel every ACL link has.
pub const SIGNALLING: u16 = 0x0001;
/// The PSM RFCOMM listens on.
pub const RFCOMM_PSM: u16 = 0x0003;
/// The first dynamically allocated channel identifier.
pub const FIRST_DYNAMIC: u16 = 0x0040;
/// The most a basic frame carries: the length is sixteen bits.
pub const MAX_PAYLOAD: usize = 65_535;

/// The connection was opened.
pub const SUCCESS: u16 = 0;
/// Nobody listens on that PSM.
pub const PSM_NOT_SUPPORTED: u16 = 2;

const CONNECTION_REQUEST: u8 = 0x02;
const CONNECTION_RESPONSE: u8 = 0x03;
const DISCONNECTION_REQUEST: u8 = 0x06;
const DISCONNECTION_RESPONSE: u8 = 0x07;

/// One basic frame on one channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub cid: u16,
    pub payload: Vec<u8>,
}

impl Frame {
    /// A frame, refusing the null identifier and more than the length says.
    ///
    /// # Errors
    /// Channel 0, or a payload over [`MAX_PAYLOAD`].
    pub fn new(cid: u16, payload: &[u8]) -> Result<Self> {
        if cid == 0 {
            return Err(protocol_error("the null channel identifier"));
        }
        if payload.len() > MAX_PAYLOAD {
            return Err(protocol_error("more than one L2CAP basic frame carries"));
        }
        Ok(Self {
            cid,
            payload: payload.to_vec(),
        })
    }

    /// The frame as the link carries it.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let length = u16::try_from(self.payload.len()).unwrap_or(u16::MAX);
        let mut out = Vec::with_capacity(4 + self.payload.len());
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&self.cid.to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    /// Exactly one frame.
    ///
    /// # Errors
    /// A frame cut off inside its header, or a length the bytes do not match.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (head, payload) = bytes
            .split_at_checked(4)
            .ok_or_else(|| protocol_error("a frame cut off inside its header"))?;
        let length = usize::from(u16::from_le_bytes([head[0], head[1]]));
        if payload.len() != length {
            return Err(protocol_error(format!(
                "a length of {length} over {} bytes",
                payload.len()
            )));
        }
        Self::new(u16::from_le_bytes([head[2], head[3]]), payload)
    }
}

/// A command on the signalling channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Open a channel to `psm`; `source` is the requester's identifier for it.
    ConnectionRequest {
        identifier: u8,
        psm: u16,
        source: u16,
    },
    /// The channel is open at `destination` on the responder — or not, per
    /// `result`.
    ConnectionResponse {
        identifier: u8,
        destination: u16,
        source: u16,
        result: u16,
    },
    DisconnectionRequest {
        identifier: u8,
        destination: u16,
        source: u16,
    },
    DisconnectionResponse {
        identifier: u8,
        destination: u16,
        source: u16,
    },
}

impl Signal {
    /// The command as channel 1 carries it: code, identifier, length, data.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let (code, identifier, words): (u8, u8, Vec<u16>) = match *self {
            Self::ConnectionRequest {
                identifier,
                psm,
                source,
            } => (CONNECTION_REQUEST, identifier, vec![psm, source]),
            Self::ConnectionResponse {
                identifier,
                destination,
                source,
                result,
            } => (
                CONNECTION_RESPONSE,
                identifier,
                vec![destination, source, result, 0],
            ),
            Self::DisconnectionRequest {
                identifier,
                destination,
                source,
            } => (DISCONNECTION_REQUEST, identifier, vec![destination, source]),
            Self::DisconnectionResponse {
                identifier,
                destination,
                source,
            } => (
                DISCONNECTION_RESPONSE,
                identifier,
                vec![destination, source],
            ),
        };
        let length = u16::try_from(words.len() * 2).unwrap_or(u16::MAX);
        let mut out = vec![code, identifier];
        out.extend_from_slice(&length.to_le_bytes());
        for word in words {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// The command at the start of a signalling payload.
    ///
    /// # Errors
    /// A code this crate does not signal, or data shorter than the code needs.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (head, data) = bytes
            .split_at_checked(4)
            .ok_or_else(|| protocol_error("a signal cut off inside its header"))?;
        let identifier = head[1];
        let word = |at: usize| -> Result<u16> {
            data.get(at * 2..at * 2 + 2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .ok_or_else(|| protocol_error("a signal shorter than its code needs"))
        };
        match head[0] {
            CONNECTION_REQUEST => Ok(Self::ConnectionRequest {
                identifier,
                psm: word(0)?,
                source: word(1)?,
            }),
            CONNECTION_RESPONSE => Ok(Self::ConnectionResponse {
                identifier,
                destination: word(0)?,
                source: word(1)?,
                result: word(2)?,
            }),
            DISCONNECTION_REQUEST => Ok(Self::DisconnectionRequest {
                identifier,
                destination: word(0)?,
                source: word(1)?,
            }),
            DISCONNECTION_RESPONSE => Ok(Self::DisconnectionResponse {
                identifier,
                destination: word(0)?,
                source: word(1)?,
            }),
            other => Err(protocol_error(format!(
                "a signalling code this crate does not carry: {other:#04x}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_basic_frame_is_length_channel_and_payload_little_endian() {
        let frame = Frame::new(0x0040, b"hello").expect("frame");
        let bytes = frame.encode();
        assert_eq!(&bytes[..4], &[5, 0, 0x40, 0]);
        assert_eq!(Frame::decode(&bytes).expect("decode"), frame);
        assert!(Frame::decode(&bytes[..3]).is_err(), "cut off");
        assert!(Frame::decode(&bytes[..8]).is_err(), "length mismatch");
        assert!(Frame::new(0, b"").is_err(), "null identifier");
        assert!(Frame::new(1, &vec![0; MAX_PAYLOAD + 1]).is_err());
        assert!(Frame::new(1, &vec![0; MAX_PAYLOAD]).is_ok());
    }

    #[test]
    fn every_signal_reads_back_as_it_was_written() {
        let signals = [
            Signal::ConnectionRequest {
                identifier: 1,
                psm: RFCOMM_PSM,
                source: 0x0040,
            },
            Signal::ConnectionResponse {
                identifier: 1,
                destination: 0x0041,
                source: 0x0040,
                result: SUCCESS,
            },
            Signal::DisconnectionRequest {
                identifier: 2,
                destination: 0x0041,
                source: 0x0040,
            },
            Signal::DisconnectionResponse {
                identifier: 2,
                destination: 0x0041,
                source: 0x0040,
            },
        ];
        for signal in signals {
            let bytes = signal.encode();
            assert_eq!(Signal::decode(&bytes).expect("decode"), signal);
            assert!(
                Signal::decode(&bytes[..5]).is_err(),
                "cut inside the first word"
            );
        }
        assert_eq!(
            Signal::ConnectionRequest {
                identifier: 1,
                psm: 3,
                source: 0x40
            }
            .encode(),
            [0x02, 0x01, 0x04, 0x00, 0x03, 0x00, 0x40, 0x00]
        );
        assert!(Signal::decode(&[0x01, 0, 0, 0]).is_err(), "command reject");
        assert!(Signal::decode(&[0x02]).is_err(), "cut off");
    }
}
