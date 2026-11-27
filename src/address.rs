//! A Bluetooth device address, its `BD_ADDR`: six octets, written as IEEE
//! 802 writes a hardware address — `70:a8:d3:36:de:ad` — and read in every
//! spelling `net::mac` reads; carried least significant octet first wherever
//! HCI and the kernel's sockets carry it (`bdaddr_t`).

use std::fmt;

use transport::error::{Result, protocol_error};

/// A device address, most significant octet first, as it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Address([u8; 6]);

impl Address {
    /// Every controller of this machine: `00:00:00:00:00:00`, the kernel's
    /// `BDADDR_ANY`, where a Receive Location listens on whichever a peer
    /// reaches.
    pub const ANY: Self = Self([0; 6]);

    /// The address `octets` writes, most significant first.
    #[must_use]
    pub const fn new(octets: [u8; 6]) -> Self {
        Self(octets)
    }

    /// The address `text` writes.
    ///
    /// # Errors
    /// Text `net::mac` does not read, or not six octets.
    pub fn parse(text: &str) -> Result<Self> {
        <[u8; 6]>::try_from(net::mac::parse(text)?)
            .map(Self)
            .map_err(|_| protocol_error(format!("{text:?} is not six octets, as a device is")))
    }

    /// As HCI and `bdaddr_t` carry it: least significant octet first.
    #[must_use]
    pub fn wire(self) -> [u8; 6] {
        let mut wire = self.0;
        wire.reverse();
        wire
    }

    /// The address HCI or `bdaddr_t` carries as `wire`.
    #[must_use]
    pub fn from_wire(mut wire: [u8; 6]) -> Self {
        wire.reverse();
        Self(wire)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&net::mac::notation(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_written_as_ieee_802_writes_it_and_carried_backwards() {
        let address = Address::parse("70-A8-D3-36-DE-AD").expect("six octets");
        assert_eq!(address.to_string(), "70:a8:d3:36:de:ad");
        assert_eq!(address.wire(), [0xad, 0xde, 0x36, 0xd3, 0xa8, 0x70]);
        assert_eq!(Address::from_wire(address.wire()), address);
        assert_eq!(Address::ANY.to_string(), "00:00:00:00:00:00");
        assert!(Address::parse("70:a8:d3:36:de").is_err(), "five octets");
        assert!(Address::parse("loopback").is_err(), "no address");
    }
}
