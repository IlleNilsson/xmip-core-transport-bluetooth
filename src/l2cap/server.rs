//! The server's side of Xmip's own L2CAP on one ACL link, packet by packet:
//! a peer's opening and closing of a channel to RFCOMM answered, and each
//! frame on it handed on. A radio a peer speaks to serves through it
//! (`channel`), and so does the in-process radio (`radio`).

use transport::error::{Result, protocol_error};

use super::{FIRST_DYNAMIC, Frame, PSM_NOT_SUPPORTED, RFCOMM_PSM, SIGNALING, SUCCESS, Signal};

/// The server's side of one ACL link: it answers a peer's connection request
/// to RFCOMM's PSM and its disconnection request on the signaling channel,
/// and hands on each frame of the channel it opened.
#[derive(Debug, Default)]
pub struct Server {
    /// The requester's identifier for the RFCOMM channel, once opened.
    remote: Option<u16>,
}

/// What one packet asked of a [`Server`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Handled {
    /// Packets for the peer, in order: the signaling's answers.
    pub answers: Vec<Vec<u8>>,
    /// The frame it carried on the open channel: an SDU for RFCOMM.
    pub sdu: Option<Vec<u8>>,
    /// The peer closed the channel.
    pub closed: bool,
}

impl Server {
    /// One packet from the peer.
    ///
    /// # Errors
    /// A packet that is not a basic frame, a frame on a channel nobody
    /// opened, or a signal where none was due.
    pub fn handle(&mut self, packet: &[u8]) -> Result<Handled> {
        let frame = Frame::decode(packet)?;
        match frame.cid {
            SIGNALING => self.signal(&Signal::decode(&frame.payload)?),
            FIRST_DYNAMIC if self.remote.is_some() => Ok(Handled {
                sdu: Some(frame.payload),
                ..Handled::default()
            }),
            other => Err(protocol_error(format!(
                "a frame on channel {other:#06x}, which nobody opened"
            ))),
        }
    }

    fn signal(&mut self, signal: &Signal) -> Result<Handled> {
        let mut closed = false;
        let answer = match *signal {
            Signal::ConnectionRequest {
                identifier,
                psm,
                source,
            } => {
                let listening = psm == RFCOMM_PSM;
                if listening {
                    self.remote = Some(source);
                }
                Signal::ConnectionResponse {
                    identifier,
                    destination: if listening { FIRST_DYNAMIC } else { 0 },
                    source,
                    result: if listening {
                        SUCCESS
                    } else {
                        PSM_NOT_SUPPORTED
                    },
                }
            }
            Signal::DisconnectionRequest {
                identifier,
                destination,
                source,
            } => {
                self.remote = None;
                closed = true;
                Signal::DisconnectionResponse {
                    identifier,
                    destination,
                    source,
                }
            }
            Signal::ConnectionResponse { .. } | Signal::DisconnectionResponse { .. } => {
                return Err(protocol_error("a response where a request was due"));
            }
        };
        Ok(Handled {
            answers: vec![Frame::new(SIGNALING, &answer.encode())?.encode()],
            sdu: None,
            closed,
        })
    }

    /// `sdu` as a frame on the channel the peer opened.
    ///
    /// # Errors
    /// No channel open.
    pub fn carried(&self, sdu: &[u8]) -> Result<Vec<u8>> {
        let remote = self
            .remote
            .ok_or_else(|| protocol_error("an RFCOMM frame before the channel opened"))?;
        Ok(Frame::new(remote, sdu)?.encode())
    }

    /// Whether a channel to RFCOMM is open.
    #[must_use]
    pub const fn open(&self) -> bool {
        self.remote.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;

    fn signal(signal: &Signal) -> Vec<u8> {
        Frame::new(SIGNALING, &signal.encode())
            .expect("frame")
            .encode()
    }

    fn answer(handled: &Handled) -> Signal {
        let frame = Frame::decode(&handled.answers[0]).expect("l2cap");
        Signal::decode(&frame.payload).expect("signal")
    }

    #[test]
    fn the_server_opens_rfcomms_channel_hands_on_its_frames_and_closes_it() {
        let mut server = Server::default();
        let connect = Signal::ConnectionRequest {
            identifier: 1,
            psm: RFCOMM_PSM,
            source: 0x0040,
        };
        let opened = server.handle(&signal(&connect)).expect("connect");
        assert_matches!(
            answer(&opened),
            Signal::ConnectionResponse {
                result: SUCCESS,
                destination: FIRST_DYNAMIC,
                ..
            }
        );
        let data = Frame::new(FIRST_DYNAMIC, b"sdu").expect("frame").encode();
        let handled = server.handle(&data).expect("data");
        assert_eq!(handled.sdu, Some(b"sdu".to_vec()));
        assert!(handled.answers.is_empty() && !handled.closed);
        let carried = Frame::decode(&server.carried(b"back").expect("open")).expect("frame");
        assert_eq!((carried.cid, carried.payload), (0x0040, b"back".to_vec()));
        let close = Signal::DisconnectionRequest {
            identifier: 2,
            destination: FIRST_DYNAMIC,
            source: 0x0040,
        };
        let closed = server.handle(&signal(&close)).expect("close");
        assert!(closed.closed);
        assert_matches!(answer(&closed), Signal::DisconnectionResponse { .. });
        assert!(server.carried(b"late").is_err(), "nothing open");
    }

    #[test]
    fn what_was_not_opened_is_refused() {
        let mut server = Server::default();
        let data = Frame::new(FIRST_DYNAMIC, b"x").expect("frame").encode();
        assert!(server.handle(&data).is_err(), "no channel");
        let elsewhere = Signal::ConnectionRequest {
            identifier: 1,
            psm: 0x0011,
            source: 0x0040,
        };
        let refused = server.handle(&signal(&elsewhere)).expect("answered");
        assert_matches!(
            answer(&refused),
            Signal::ConnectionResponse {
                result: PSM_NOT_SUPPORTED,
                ..
            }
        );
        assert!(server.handle(&data).is_err(), "still no channel");
        assert!(server.handle(&[0, 0, 0x99, 0]).is_err(), "unknown channel");
        let reply = Signal::ConnectionResponse {
            identifier: 1,
            destination: 1,
            source: 1,
            result: 0,
        };
        assert!(
            server.handle(&signal(&reply)).is_err(),
            "a response to nothing"
        );
    }
}
