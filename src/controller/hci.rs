//! What a controller and its host say over HCI, the Host Controller
//! Interface of the Bluetooth Core Specification, as far as the kernel
//! says it to a virtual controller of `controller`: packet types, the
//! commands answered, the events sent, and what a controller returns to each
//! command the kernel reads it with as it powers one on.
//!
//! The controller is BR/EDR alone: every feature bit clear, so the kernel
//! asks for no Secure Simple Pairing, no LE and no extended feature page,
//! and a channel at L2CAP's low security opens with no pairing; and no
//! optional command supported, so it reads none of them.

/// A command, host to controller.
pub const COMMAND: u8 = 0x01;
/// ACL data, either way.
pub const ACL: u8 = 0x02;
/// An event, controller to host.
pub const EVENT: u8 = 0x04;
/// The vhci driver's own packet: the device it made, and its index.
pub const VENDOR: u8 = 0xff;

pub const CONNECTION_COMPLETE: u8 = 0x03;
pub const CONNECTION_REQUEST: u8 = 0x04;
pub const DISCONNECTION_COMPLETE: u8 = 0x05;
pub const REMOTE_NAME_COMPLETE: u8 = 0x07;
pub const REMOTE_FEATURES_COMPLETE: u8 = 0x0b;
const COMMAND_COMPLETE: u8 = 0x0e;
const COMMAND_STATUS: u8 = 0x0f;
pub const COMPLETED_PACKETS: u8 = 0x13;
pub const CLOCK_OFFSET_COMPLETE: u8 = 0x1c;

pub const CREATE_CONNECTION: u16 = 0x0405;
pub const DISCONNECT: u16 = 0x0406;
pub const ACCEPT_CONNECTION: u16 = 0x0409;
pub const REJECT_CONNECTION: u16 = 0x040a;
pub const REMOTE_NAME_REQUEST: u16 = 0x0419;
pub const READ_REMOTE_FEATURES: u16 = 0x041b;
pub const READ_CLOCK_OFFSET: u16 = 0x041f;
pub const WRITE_LOCAL_NAME: u16 = 0x0c13;
pub const WRITE_SCAN_ENABLE: u16 = 0x0c1a;

pub const SUCCESS: u8 = 0x00;
pub const UNKNOWN_COMMAND: u8 = 0x01;
pub const PAGE_TIMEOUT: u8 = 0x04;
pub const LOCAL_HOST_TERMINATED: u8 = 0x16;
/// Why a connection request is rejected where the host gives none.
pub const LIMITED_RESOURCES: u8 = 0x0d;

/// Write Scan Enable's page scan: the controller answers a page, so a peer
/// may connect.
pub const PAGE_SCAN: u8 = 0x02;

/// The link type an ACL connection is.
pub const ACL_LINK: u8 = 0x01;

/// The name each controller gives: what a remote name request reads too.
pub const NAME: &[u8] = b"Xmip virtual controller";

/// An event of `code` with `parameters`.
#[must_use]
pub fn event(code: u8, parameters: &[u8]) -> Vec<u8> {
    let mut packet = vec![
        EVENT,
        code,
        u8::try_from(parameters.len()).unwrap_or(u8::MAX),
    ];
    packet.extend_from_slice(parameters);
    packet
}

/// Command Complete for `opcode`, `returned` after it: its status first.
#[must_use]
pub fn complete(opcode: u16, returned: &[u8]) -> Vec<u8> {
    let mut parameters = vec![1];
    parameters.extend(opcode.to_le_bytes());
    parameters.extend_from_slice(returned);
    event(COMMAND_COMPLETE, &parameters)
}

/// Command Status for `opcode`: begun, or refused with `status`.
#[must_use]
pub fn status(opcode: u16, status: u8) -> Vec<u8> {
    let [low, high] = opcode.to_le_bytes();
    event(COMMAND_STATUS, &[status, 1, low, high])
}

/// The name, as HCI carries it: 248 octets, zero-padded.
#[must_use]
pub fn name() -> Vec<u8> {
    let mut name = NAME.to_vec();
    name.resize(248, 0);
    name
}

/// The group a command is in: its opcode's upper six bits.
#[must_use]
pub const fn group(opcode: u16) -> u16 {
    opcode >> 10
}

/// Link control's group: a connection's commands, each begun with Command
/// Status and finished by an event.
pub const LINK_CONTROL: u16 = 0x01;

/// What a controller whose address is `wire` returns after the status of
/// each command the kernel reads it with as it powers it on, or that a host
/// reads it with later; `None` for one it does not know.
#[must_use]
pub fn read(opcode: u16, wire: [u8; 6], parameters: &[u8]) -> Option<Vec<u8>> {
    Some(match opcode {
        // Local Version: HCI and LMP 4.0, manufacturer 0xffff, the SIG's
        // for tests and internal use.
        0x1001 => vec![0x06, 0, 0, 0x06, 0xff, 0xff, 0, 0],
        // Local Supported Commands: none beyond what every controller has.
        0x1002 => vec![0; 64],
        // Local Supported Features: BR/EDR, and nothing more.
        0x1003 => vec![0; 8],
        // Local Extended Features: the page asked, the only one.
        0x1004 => vec![
            parameters.first().copied().unwrap_or(0),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ],
        // Buffer Size: 1021-octet ACL packets, eight of them; 64-octet SCO.
        0x1005 => vec![0xfd, 0x03, 64, 8, 0, 8, 0],
        0x1009 => wire.to_vec(),
        0x0c14 => name(),
        // Class of Device, none; Voice Setting, the specification's default.
        0x0c23 => vec![0; 3],
        0x0c25 => vec![0x60, 0],
        // One inquiry access code supported, the general one in use.
        0x0c38 => vec![1],
        0x0c39 => vec![1, 0x33, 0x8b, 0x9e],
        // Stored link keys: none kept, none deleted.
        0x0c0d => vec![0, 0, 0, 0],
        0x0c12 => vec![0, 0],
        // Page Scan Activity and Type, Inquiry Response Transmit Power.
        0x0c1b => vec![0, 0x08, 0x12, 0],
        0x0c46 | 0x0c58 => vec![0],
        _ => return None,
    })
}
