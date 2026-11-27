//! One connection's RFCOMM, on any L2CAP channel (`link::Channel`): the
//! client's dialogue that writes a Stream as one data link ([`sent`]), and
//! the server's session that answers it and takes the Stream whole as the
//! link disconnects ([`Session`], [`served`]). The same on the kernel's
//! channel and on Xmip's own L2CAP in the in-process radio.

use transport::TransportError;
use transport::error::{Result, protocol_error};

use crate::link::Channel;
use crate::rfcomm::{self, Control};

/// What the session has to say back to one frame, and what it has taken
/// whole.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Answered {
    /// Frames for the client, in order.
    pub answers: Vec<Vec<u8>>,
    /// A Stream that just closed: the DLCI it came on and its bytes.
    pub complete: Option<(u8, Vec<u8>)>,
}

/// One connection's state on the server.
#[derive(Debug, Default)]
pub struct Session {
    multiplexer: bool,
    open: Option<u8>,
    information: Vec<u8>,
}

impl Session {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// One frame from the client, answered: SABM and DISC with UA, or DM
    /// where the multiplexer or the link is not open; UIH taken, and not
    /// acknowledged.
    ///
    /// # Errors
    /// A frame RFCOMM cannot read.
    pub fn answer(&mut self, sdu: &[u8]) -> Result<Answered> {
        let frame = rfcomm::Frame::decode(sdu)?;
        let mut answered = Answered::default();
        let control = match frame.control {
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
                return Ok(answered);
            }
            Control::Disc if self.open == Some(frame.dlci) => {
                self.open = None;
                answered.complete = Some((frame.dlci, std::mem::take(&mut self.information)));
                Control::Ua
            }
            Control::Disc if frame.dlci == 0 => {
                self.multiplexer = false;
                Control::Ua
            }
            Control::Sabm | Control::Uih | Control::Disc => Control::Dm,
            Control::Ua | Control::Dm => return Ok(answered),
        };
        answered
            .answers
            .push(rfcomm::Frame::command(frame.dlci, control)?.encode());
        Ok(answered)
    }
}

/// Serve one connection on `channel` until the client closes it: the Stream
/// its last data link carried whole, with the link's DLCI.
///
/// # Errors
/// A client that went quiet, broke RFCOMM, or a channel that failed.
pub fn served(channel: &mut dyn Channel) -> Result<Option<(u8, Vec<u8>)>> {
    let mut session = Session::new();
    let mut taken = None;
    while let Some(sdu) = channel.receive()? {
        let answered = session.answer(&sdu)?;
        for answer in &answered.answers {
            channel.send(answer)?;
        }
        if answered.complete.is_some() {
            taken = answered.complete;
        }
    }
    Ok(taken)
}

/// Write `bytes` as one data link on server channel `server`: the
/// multiplexer and the link opened with SABM, the Stream in UIH frames of N1
/// bytes, the link and the multiplexer closed with DISC, each command
/// answered with UA.
///
/// # Errors
/// A server that does not answer, closes the channel, refuses the link or
/// answers something else.
pub fn sent(channel: &mut dyn Channel, server: u8, bytes: &[u8]) -> Result<()> {
    let dlci = server << 1;
    command(channel, 0, Control::Sabm)?;
    command(channel, dlci, Control::Sabm)?;
    for chunk in bytes.chunks(rfcomm::N1) {
        channel.send(&rfcomm::Frame::new(dlci, Control::Uih, chunk)?.encode())?;
    }
    command(channel, dlci, Control::Disc)?;
    command(channel, 0, Control::Disc)
}

/// `control` on `dlci`, answered with UA.
fn command(channel: &mut dyn Channel, dlci: u8, control: Control) -> Result<()> {
    channel.send(&rfcomm::Frame::command(dlci, control)?.encode())?;
    let answer = channel
        .receive()?
        .ok_or_else(|| TransportError::retryable("the peer closed the channel"))?;
    match rfcomm::Frame::decode(&answer)?.control {
        Control::Ua => Ok(()),
        Control::Dm => Err(protocol_error(format!("DLCI {dlci} is not open to us"))),
        other => Err(protocol_error(format!("{other:?} where UA was due"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn frame(dlci: u8, control: Control, information: &[u8]) -> Vec<u8> {
        rfcomm::Frame::new(dlci, control, information)
            .expect("frame")
            .encode()
    }

    fn answered(answered: &Answered) -> Control {
        rfcomm::Frame::decode(&answered.answers[0])
            .expect("rfcomm")
            .control
    }

    #[test]
    fn a_stream_is_the_information_between_sabm_and_disc() {
        let mut session = Session::new();
        let sabm = session.answer(&frame(0, Control::Sabm, &[])).expect("0");
        assert_eq!(answered(&sabm), Control::Ua);
        let sabm = session.answer(&frame(2, Control::Sabm, &[])).expect("2");
        assert_eq!(answered(&sabm), Control::Ua);
        for chunk in [&b"hel"[..], b"lo"] {
            let uih = session.answer(&frame(2, Control::Uih, chunk)).expect("uih");
            assert!(uih.answers.is_empty(), "data is not acknowledged");
        }
        let disc = session.answer(&frame(2, Control::Disc, &[])).expect("disc");
        assert_eq!(answered(&disc), Control::Ua);
        assert_eq!(disc.complete, Some((2, b"hello".to_vec())));
        let disc = session.answer(&frame(0, Control::Disc, &[])).expect("0");
        assert_eq!(answered(&disc), Control::Ua);
    }

    #[test]
    fn what_was_not_opened_is_answered_dm() {
        let mut session = Session::new();
        let early = session.answer(&frame(2, Control::Sabm, &[])).expect("2");
        assert_eq!(answered(&early), Control::Dm, "multiplexer not up");
        let data = session.answer(&frame(2, Control::Uih, b"x")).expect("data");
        assert_eq!(answered(&data), Control::Dm, "link not open");
        assert!(session.answer(&[0x0b]).is_err(), "not a frame");
    }

    /// A channel whose server is a session in this process.
    struct Inside(Session, VecDeque<Vec<u8>>, Option<(u8, Vec<u8>)>);

    impl Channel for Inside {
        fn send(&mut self, sdu: &[u8]) -> Result<()> {
            let answered = self.0.answer(sdu)?;
            self.1.extend(answered.answers);
            self.2 = answered.complete.or(self.2.take());
            Ok(())
        }
        fn receive(&mut self) -> Result<Option<Vec<u8>>> {
            Ok(self.1.pop_front())
        }
        fn close(self: Box<Self>) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_dialogue_writes_a_stream_the_session_takes_whole() {
        let mut inside = Inside(Session::new(), VecDeque::new(), None);
        let bytes: Vec<u8> = (0..=255).cycle().take(rfcomm::N1 * 3 + 1).collect();
        sent(&mut inside, 3, &bytes).expect("sent");
        assert_eq!(inside.2, Some((6, bytes)));
        assert!(
            sent(&mut inside, 3, b"").is_ok(),
            "an empty Stream is a link with no UIH frames"
        );
    }
}
