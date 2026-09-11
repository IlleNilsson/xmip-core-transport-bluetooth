//! The RFCOMM server's side of one connection: it answers the L2CAP
//! connection request, the multiplexer's SABM and each data link's, takes
//! the UIH frames of the data link and hands the Stream over when the link
//! disconnects. The same session runs inside the loopback radio and behind
//! a real one.

use transport::error::{Result, protocol_error};

use crate::l2cap::{self, Signal};
use crate::rfcomm::{self, Control};

/// What the session has to say back, and what it has taken whole.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// Packets for the other end, in order.
    pub answers: Vec<Vec<u8>>,
    /// A Stream that just closed: the DLCI it came on and its bytes.
    pub complete: Option<(u8, Vec<u8>)>,
    /// The other end closed the L2CAP channel: the connection is over.
    pub closed: bool,
}

/// One connection's state on the server.
#[derive(Debug, Default)]
pub struct Session {
    /// The requester's identifier for the RFCOMM channel, once opened.
    remote: Option<u16>,
    multiplexer: bool,
    open: Option<u8>,
    information: Vec<u8>,
}

impl Session {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// One packet from the other end, answered.
    ///
    /// # Errors
    /// A packet that is not an L2CAP frame, a frame on a channel nobody
    /// opened, or a frame RFCOMM cannot read.
    pub fn handle(&mut self, packet: &[u8]) -> Result<Response> {
        let frame = l2cap::Frame::decode(packet)?;
        match frame.cid {
            l2cap::SIGNALLING => self.signal(&Signal::decode(&frame.payload)?),
            l2cap::FIRST_DYNAMIC if self.remote.is_some() => {
                self.rfcomm(&rfcomm::Frame::decode(&frame.payload)?)
            }
            other => Err(protocol_error(format!(
                "a frame on channel {other:#06x}, which nobody opened"
            ))),
        }
    }

    fn signal(&mut self, signal: &Signal) -> Result<Response> {
        let mut closed = false;
        let answer = match *signal {
            Signal::ConnectionRequest {
                identifier,
                psm,
                source,
            } => {
                let listening = psm == l2cap::RFCOMM_PSM;
                if listening {
                    self.remote = Some(source);
                }
                Signal::ConnectionResponse {
                    identifier,
                    destination: if listening { l2cap::FIRST_DYNAMIC } else { 0 },
                    source,
                    result: if listening {
                        l2cap::SUCCESS
                    } else {
                        l2cap::PSM_NOT_SUPPORTED
                    },
                }
            }
            Signal::DisconnectionRequest {
                identifier,
                destination,
                source,
            } => {
                *self = Self::new();
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
        Ok(Response {
            answers: vec![l2cap::Frame::new(l2cap::SIGNALLING, &answer.encode())?.encode()],
            complete: None,
            closed,
        })
    }

    fn rfcomm(&mut self, frame: &rfcomm::Frame) -> Result<Response> {
        let mut response = Response::default();
        let answer = match frame.control {
            Control::Sabm if frame.dlci == 0 => {
                self.multiplexer = true;
                Control::Ua
            }
            Control::Sabm if self.multiplexer && self.open.is_none() => {
                self.open = Some(frame.dlci);
                self.information.clear();
                Control::Ua
            }
            Control::Uih if self.open == Some(frame.dlci) => {
                self.information.extend_from_slice(&frame.information);
                return Ok(response);
            }
            Control::Disc if self.open == Some(frame.dlci) => {
                self.open = None;
                response.complete = Some((frame.dlci, std::mem::take(&mut self.information)));
                Control::Ua
            }
            Control::Disc if frame.dlci == 0 => {
                self.multiplexer = false;
                Control::Ua
            }
            Control::Sabm | Control::Uih | Control::Disc => Control::Dm,
            Control::Ua | Control::Dm => return Ok(response),
        };
        let remote = self
            .remote
            .ok_or_else(|| protocol_error("an RFCOMM frame before the channel opened"))?;
        let answer = rfcomm::Frame::command(frame.dlci, answer)?;
        response
            .answers
            .push(l2cap::Frame::new(remote, &answer.encode())?.encode());
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(signal: &Signal) -> Vec<u8> {
        l2cap::Frame::new(l2cap::SIGNALLING, &signal.encode())
            .expect("frame")
            .encode()
    }

    fn frame(dlci: u8, control: Control, information: &[u8]) -> Vec<u8> {
        let frame = rfcomm::Frame::new(dlci, control, information).expect("frame");
        l2cap::Frame::new(l2cap::FIRST_DYNAMIC, &frame.encode())
            .expect("frame")
            .encode()
    }

    fn answered(response: &Response) -> rfcomm::Frame {
        let packet = l2cap::Frame::decode(&response.answers[0]).expect("l2cap");
        assert_eq!(packet.cid, 0x0040, "on the requester's channel");
        rfcomm::Frame::decode(&packet.payload).expect("rfcomm")
    }

    #[test]
    fn a_stream_is_the_information_between_sabm_and_disc() {
        let mut session = Session::new();
        let connect = Signal::ConnectionRequest {
            identifier: 1,
            psm: l2cap::RFCOMM_PSM,
            source: 0x0040,
        };
        let response = session.handle(&signal(&connect)).expect("connect");
        let packet = l2cap::Frame::decode(&response.answers[0]).expect("l2cap");
        assert!(matches!(
            Signal::decode(&packet.payload).expect("signal"),
            Signal::ConnectionResponse {
                result: 0,
                destination: 0x0040,
                ..
            }
        ));
        let response = session
            .handle(&frame(0, Control::Sabm, &[]))
            .expect("sabm 0");
        assert_eq!(answered(&response).control, Control::Ua);
        let response = session
            .handle(&frame(2, Control::Sabm, &[]))
            .expect("sabm 2");
        assert_eq!(answered(&response).control, Control::Ua);
        for chunk in [&b"hel"[..], b"lo"] {
            let response = session.handle(&frame(2, Control::Uih, chunk)).expect("uih");
            assert!(response.answers.is_empty(), "data is not acknowledged");
        }
        let response = session.handle(&frame(2, Control::Disc, &[])).expect("disc");
        assert_eq!(answered(&response).control, Control::Ua);
        assert_eq!(response.complete, Some((2, b"hello".to_vec())));
        let response = session
            .handle(&frame(0, Control::Disc, &[]))
            .expect("disc 0");
        assert_eq!(answered(&response).control, Control::Ua);
    }

    #[test]
    fn what_was_not_opened_is_answered_dm_or_refused() {
        let mut session = Session::new();
        assert!(
            session.handle(&frame(2, Control::Uih, b"x")).is_err(),
            "no channel"
        );
        let elsewhere = Signal::ConnectionRequest {
            identifier: 1,
            psm: 0x0011,
            source: 0x0040,
        };
        let response = session.handle(&signal(&elsewhere)).expect("answered");
        let packet = l2cap::Frame::decode(&response.answers[0]).expect("l2cap");
        assert!(matches!(
            Signal::decode(&packet.payload).expect("signal"),
            Signal::ConnectionResponse { result: 2, .. }
        ));
        assert!(
            session.handle(&frame(2, Control::Uih, b"x")).is_err(),
            "still no channel"
        );
        let connect = Signal::ConnectionRequest {
            identifier: 2,
            psm: l2cap::RFCOMM_PSM,
            source: 0x0040,
        };
        session.handle(&signal(&connect)).expect("connect");
        let response = session
            .handle(&frame(2, Control::Sabm, &[]))
            .expect("before 0");
        assert_eq!(
            answered(&response).control,
            Control::Dm,
            "multiplexer not up"
        );
        let response = session.handle(&frame(2, Control::Uih, b"x")).expect("data");
        assert_eq!(answered(&response).control, Control::Dm, "link not open");
        assert!(session.handle(&[0, 0, 0x99, 0]).is_err(), "unknown channel");
        let reply = Signal::ConnectionResponse {
            identifier: 1,
            destination: 1,
            source: 1,
            result: 0,
        };
        assert!(
            session.handle(&signal(&reply)).is_err(),
            "a response to nothing"
        );
    }
}
