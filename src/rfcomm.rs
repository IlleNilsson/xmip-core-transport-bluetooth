//! The RFCOMM frame, TS 07.10 as Bluetooth takes it: an address byte naming
//! the data link connection, a control byte, one or two length bytes, the
//! information and a frame check sequence. DLCI 0 is the multiplexer's
//! control channel; a server channel `n` is DLCI `2n` seen from the side
//! that started the multiplexer.

use transport::error::{Result, protocol_error};

/// The default maximum frame size, N1: what a UIH frame carries unless the
/// parameters were negotiated.
pub const N1: usize = 127;
/// The most a frame can say it carries: the length is fifteen bits.
pub const MAX_FRAME: usize = 32_767;
/// The highest DLCI: six bits, and 62 and 63 are reserved.
pub const MAX_DLCI: u8 = 61;

/// Address byte: extension bit set, command/response bit set.
const EA_CR: u8 = 0x03;
/// The poll/final bit in the control byte.
const POLL_FINAL: u8 = 0x10;

/// The control byte's meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// Set asynchronous balanced mode: open the DLC.
    Sabm,
    /// Unnumbered acknowledgement: the answer to SABM and DISC.
    Ua,
    /// Disconnected mode: the DLC is not, and will not be, open.
    Dm,
    /// Disconnect: close the DLC.
    Disc,
    /// Unnumbered information with header check: the data.
    Uih,
}

impl Control {
    const fn byte(self) -> u8 {
        match self {
            Self::Sabm => 0x2f | POLL_FINAL,
            Self::Ua => 0x63 | POLL_FINAL,
            Self::Dm => 0x0f | POLL_FINAL,
            Self::Disc => 0x43 | POLL_FINAL,
            Self::Uih => 0xef,
        }
    }

    fn from_byte(byte: u8) -> Result<Self> {
        match byte & !POLL_FINAL {
            0x2f => Ok(Self::Sabm),
            0x63 => Ok(Self::Ua),
            0x0f => Ok(Self::Dm),
            0x43 => Ok(Self::Disc),
            0xef => Ok(Self::Uih),
            other => Err(protocol_error(format!(
                "a control byte RFCOMM does not use: {other:#04x}"
            ))),
        }
    }
}

/// One frame on one DLCI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub dlci: u8,
    pub control: Control,
    pub information: Vec<u8>,
}

impl Frame {
    /// A frame, refusing a DLCI the six bits do not name and more
    /// information than the length can say.
    ///
    /// # Errors
    /// A DLCI over [`MAX_DLCI`], or information over [`MAX_FRAME`].
    pub fn new(dlci: u8, control: Control, information: &[u8]) -> Result<Self> {
        if dlci > MAX_DLCI {
            return Err(protocol_error("a DLCI over 61"));
        }
        if information.len() > MAX_FRAME {
            return Err(protocol_error("more than one RFCOMM frame can say"));
        }
        Ok(Self {
            dlci,
            control,
            information: information.to_vec(),
        })
    }

    /// A frame with no information: SABM, UA, DM, DISC.
    ///
    /// # Errors
    /// A DLCI over [`MAX_DLCI`].
    pub fn command(dlci: u8, control: Control) -> Result<Self> {
        Self::new(dlci, control, &[])
    }

    /// The frame as the L2CAP channel carries it.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![(self.dlci << 2) | EA_CR, self.control.byte()];
        let length = self.information.len();
        if length <= 127 {
            out.push((u8::try_from(length).unwrap_or(u8::MAX) << 1) | 1);
        } else {
            out.push(u8::try_from(length & 0x7f).unwrap_or(0) << 1);
            out.push(u8::try_from(length >> 7).unwrap_or(u8::MAX));
        }
        let checked = if self.control == Control::Uih {
            2
        } else {
            out.len()
        };
        let check = fcs(&out[..checked]);
        out.extend_from_slice(&self.information);
        out.push(check);
        out
    }

    /// Exactly one frame, its check sequence checked.
    ///
    /// # Errors
    /// A frame cut off, a control byte RFCOMM does not use, a length the
    /// bytes do not match, or a check sequence that does not check.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let cut = || protocol_error("a frame cut off before its check sequence");
        let address = *bytes.first().ok_or_else(cut)?;
        let control = Control::from_byte(*bytes.get(1).ok_or_else(cut)?)?;
        let first = *bytes.get(2).ok_or_else(cut)?;
        let (length, at) = if first & 1 != 0 {
            (usize::from(first >> 1), 3)
        } else {
            let second = *bytes.get(3).ok_or_else(cut)?;
            (usize::from(first >> 1) | (usize::from(second) << 7), 4)
        };
        let information = bytes.get(at..at + length).ok_or_else(cut)?;
        let check = *bytes.get(at + length).ok_or_else(cut)?;
        if bytes.len() != at + length + 1 {
            return Err(protocol_error("bytes after the check sequence"));
        }
        let checked = if control == Control::Uih { 2 } else { at };
        if fcs(&bytes[..checked]) != check {
            return Err(protocol_error("a check sequence that does not check"));
        }
        Self::new(address >> 2, control, information)
    }
}

/// The frame check sequence: the CRC-8 of TS 07.10, reflected, seeded with
/// ones and complemented — over the address and control bytes of a UIH
/// frame, and the length bytes too of any other.
#[must_use]
pub fn fcs(bytes: &[u8]) -> u8 {
    let crc = bytes.iter().fold(0xffu8, |mut crc, byte| {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xe0
            } else {
                crc >> 1
            };
        }
        crc
    });
    0xff - crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sabm_on_the_control_channel_is_the_four_bytes_the_specification_shows() {
        let sabm = Frame::command(0, Control::Sabm).expect("frame");
        assert_eq!(sabm.encode(), [0x03, 0x3f, 0x01, 0x1c]);
        let ua = Frame::command(0, Control::Ua).expect("frame");
        assert_eq!(ua.encode(), [0x03, 0x73, 0x01, 0xd7]);
        assert_eq!(Frame::decode(&sabm.encode()).expect("decode"), sabm);
    }

    #[test]
    fn a_uih_frame_checks_only_its_address_and_control() {
        let uih = Frame::new(2, Control::Uih, b"data").expect("frame");
        let bytes = uih.encode();
        assert_eq!(&bytes[..3], &[0x0b, 0xef, 0x09]);
        assert_eq!(bytes[bytes.len() - 1], fcs(&[0x0b, 0xef]));
        assert_eq!(Frame::decode(&bytes).expect("decode"), uih);
        let long = Frame::new(2, Control::Uih, &[7; 300]).expect("frame");
        let bytes = long.encode();
        assert_eq!(
            &bytes[2..4],
            &[0x58, 0x02],
            "two length bytes: 300 = 44 + 2 * 128"
        );
        assert_eq!(Frame::decode(&bytes).expect("decode"), long);
    }

    #[test]
    fn what_is_not_a_frame_is_refused() {
        let bytes = Frame::new(4, Control::Uih, b"x").expect("frame").encode();
        assert!(Frame::decode(&bytes[..2]).is_err(), "cut off");
        let mut bad = bytes.clone();
        bad[1] = 0x00;
        assert!(Frame::decode(&bad).is_err(), "control byte");
        let mut bad = bytes.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(Frame::decode(&bad).is_err(), "check sequence");
        let mut trailing = bytes;
        trailing.push(0);
        assert!(Frame::decode(&trailing).is_err(), "bytes after");
        assert!(Frame::command(62, Control::Sabm).is_err(), "reserved DLCI");
        assert!(Frame::new(2, Control::Uih, &vec![0; MAX_FRAME + 1]).is_err());
    }
}
