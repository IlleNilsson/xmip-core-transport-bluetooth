//! The air between two virtual controllers: each controller's `/dev/vhci`,
//! read on a thread of its own, and every packet the kernel writes to one
//! answered as a controller answers it (`hci`) — a command with its
//! completion or its status and the events after, a page carried to the
//! other controller, ACL data handed to the other host on its own handle
//! for the link and counted done to the sender's.

use std::fs::{File, OpenOptions};
use std::io::{self, PipeReader, Read, Write};
use std::sync::{Condvar, Mutex};

use rustix::fd::AsFd;
use transport::error::{Result, classify};
use transport::socket::ready;
use xcore::lock::Locked;

use super::hci::{self, PAGE_SCAN, SUCCESS};
use crate::address::Address;

/// The kernel's virtual controller driver.
pub const VHCI: &str = "/dev/vhci";

/// Two controllers, what is known of them, and a wake for whoever waits on
/// it.
pub struct Air {
    pub ends: [End; 2],
    pub state: Mutex<State>,
    pub changed: Condvar,
}

/// One controller: the driver's file the kernel speaks to it on, and the
/// address drawn for it.
pub struct End {
    file: File,
    pub address: Address,
}

#[derive(Default)]
pub struct State {
    /// Each controller's index, `hci<index>`, once the kernel said it.
    pub index: [Option<u16>; 2],
    /// Whether the kernel powered each on: it wrote the controller's name.
    pub powered: [bool; 2],
    /// Each controller's scan, as the kernel last wrote it.
    pub scan: [u8; 2],
    /// Each ACL link: its handle on each controller.
    links: Vec<[u16; 2]>,
    handles: u16,
    /// Commands answered as unknown, for a failure to name.
    pub unknown: Vec<u16>,
    /// Why the air stopped.
    pub failed: Option<String>,
}

impl End {
    /// A controller the kernel makes, at an address drawn for it.
    pub fn made() -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(VHCI)
            .map_err(|error| classify(&format!("opening {VHCI}"), &error))?;
        file.write_all(&[hci::VENDOR, 0x00])
            .map_err(|error| classify("asking for a primary controller", &error))?;
        let address = Address::new(codec::random::array());
        Ok(Self { file, address })
    }
}

impl Air {
    /// What the kernel says to controller `end`, answered until `stopped`
    /// is closed.
    pub fn carry(&self, end: usize, stopped: &PipeReader) {
        let mut buffer = vec![0; 4096];
        let mut file = &self.ends[end].file;
        loop {
            match ready(&[file.as_fd(), stopped.as_fd()], None) {
                Ok(woke) if woke.get(1).copied().unwrap_or(true) => return,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return self.fail(format!("waiting on {VHCI}: {error}")),
            }
            let heard = match file.read(&mut buffer) {
                Ok(0) => return self.fail(format!("{VHCI} closed")),
                Ok(heard) => heard,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return self.fail(format!("reading {VHCI}: {error}")),
            };
            if let Err(error) = self.heard(end, &buffer[..heard]) {
                return self.fail(format!("answering the kernel: {error}"));
            }
        }
    }

    fn fail(&self, why: String) {
        self.state.locked().failed.get_or_insert(why);
        self.changed.notify_all();
    }

    fn send(&self, end: usize, packet: &[u8]) -> io::Result<()> {
        let mut file = &self.ends[end].file;
        file.write_all(packet)
    }

    fn heard(&self, end: usize, packet: &[u8]) -> io::Result<()> {
        let mut state = self.state.locked();
        match packet {
            [hci::VENDOR, _, low, high] => {
                state.index[end] = Some(u16::from_le_bytes([*low, *high]));
                self.changed.notify_all();
                Ok(())
            }
            [hci::COMMAND, low, high, _, parameters @ ..] => {
                let opcode = u16::from_le_bytes([*low, *high]);
                self.command(&mut state, end, opcode, parameters)
            }
            [hci::ACL, data @ ..] => self.acl(&state, end, data),
            _ => Ok(()),
        }
    }

    fn command(
        &self,
        state: &mut State,
        end: usize,
        opcode: u16,
        parameters: &[u8],
    ) -> io::Result<()> {
        let other = 1 - end;
        match opcode {
            hci::CREATE_CONNECTION => self.page(state, end, parameters),
            hci::ACCEPT_CONNECTION => self.connect(state, end),
            hci::REJECT_CONNECTION => {
                self.send(end, &hci::status(opcode, SUCCESS))?;
                let reason = parameters.get(6).copied();
                let reason = reason.unwrap_or(hci::LIMITED_RESOURCES);
                let refused = connection_complete(reason, 0, self.ends[end].address);
                self.send(other, &refused)
            }
            hci::DISCONNECT => self.disconnect(state, end, parameters),
            hci::READ_REMOTE_FEATURES | hci::READ_CLOCK_OFFSET => {
                self.send(end, &hci::status(opcode, SUCCESS))?;
                let (code, zeros) = if opcode == hci::READ_REMOTE_FEATURES {
                    (hci::REMOTE_FEATURES_COMPLETE, 8)
                } else {
                    (hci::CLOCK_OFFSET_COMPLETE, 2)
                };
                let mut done = vec![SUCCESS];
                done.extend(parameters.get(..2).unwrap_or(&[0, 0]));
                done.resize(done.len() + zeros, 0);
                self.send(end, &hci::event(code, &done))
            }
            hci::REMOTE_NAME_REQUEST => {
                self.send(end, &hci::status(opcode, SUCCESS))?;
                let asked = parameters.get(..6).unwrap_or(&[0; 6]);
                let known = asked == self.ends[other].address.wire();
                let mut done = vec![if known { SUCCESS } else { hci::PAGE_TIMEOUT }];
                done.extend(asked);
                done.extend(hci::name());
                self.send(end, &hci::event(hci::REMOTE_NAME_COMPLETE, &done))
            }
            hci::WRITE_SCAN_ENABLE | hci::WRITE_LOCAL_NAME => {
                if opcode == hci::WRITE_SCAN_ENABLE {
                    state.scan[end] = parameters.first().copied().unwrap_or(0);
                } else {
                    state.powered[end] = true;
                }
                self.changed.notify_all();
                self.send(end, &hci::complete(opcode, &[SUCCESS]))
            }
            _ if hci::group(opcode) == hci::LINK_CONTROL => {
                state.unknown.push(opcode);
                self.send(end, &hci::status(opcode, hci::UNKNOWN_COMMAND))
            }
            _ => {
                let wire = self.ends[end].address.wire();
                let returned = match hci::read(opcode, wire, parameters) {
                    Some(read) => [&[SUCCESS][..], &read].concat(),
                    // Link policy's and the controller's settings: taken.
                    None if matches!(hci::group(opcode), 0x02 | 0x03) => vec![SUCCESS],
                    None => {
                        state.unknown.push(opcode);
                        vec![hci::UNKNOWN_COMMAND]
                    }
                };
                self.send(end, &hci::complete(opcode, &returned))
            }
        }
    }

    /// A page from `end`: a connection request on the other controller
    /// where it is the device paged and answers pages, a page timeout else.
    fn page(&self, state: &State, end: usize, parameters: &[u8]) -> io::Result<()> {
        let other = 1 - end;
        self.send(end, &hci::status(hci::CREATE_CONNECTION, SUCCESS))?;
        let mut paged = [0; 6];
        paged.copy_from_slice(parameters.get(..6).unwrap_or(&[0; 6]));
        if paged == self.ends[other].address.wire() && state.scan[other] & PAGE_SCAN != 0 {
            let mut request = self.ends[end].address.wire().to_vec();
            request.extend([0, 0, 0, hci::ACL_LINK]);
            return self.send(other, &hci::event(hci::CONNECTION_REQUEST, &request));
        }
        let timed_out = connection_complete(hci::PAGE_TIMEOUT, 0, Address::from_wire(paged));
        self.send(end, &timed_out)
    }

    /// The page `end` accepted: an ACL link, a handle on each controller.
    fn connect(&self, state: &mut State, end: usize) -> io::Result<()> {
        let other = 1 - end;
        self.send(end, &hci::status(hci::ACCEPT_CONNECTION, SUCCESS))?;
        let mut link = [0; 2];
        for side in [end, other] {
            state.handles += 1;
            link[side] = state.handles;
        }
        state.links.push(link);
        let (here, there) = (self.ends[end].address, self.ends[other].address);
        self.send(end, &connection_complete(SUCCESS, link[end], there))?;
        self.send(other, &connection_complete(SUCCESS, link[other], here))
    }

    fn disconnect(&self, state: &mut State, end: usize, parameters: &[u8]) -> io::Result<()> {
        self.send(end, &hci::status(hci::DISCONNECT, SUCCESS))?;
        let handle = handle_of(parameters);
        let reason = parameters.get(2).copied();
        let reason = reason.unwrap_or(hci::LOCAL_HOST_TERMINATED);
        let Some(at) = state.links.iter().position(|link| link[end] == handle) else {
            return Ok(());
        };
        let link = state.links.remove(at);
        let local = disconnection_complete(link[end], hci::LOCAL_HOST_TERMINATED);
        self.send(end, &local)?;
        self.send(1 - end, &disconnection_complete(link[1 - end], reason))
    }

    /// An ACL packet from `end`'s host: to the other host on its handle for
    /// the link, and the packet counted done to `end`'s.
    fn acl(&self, state: &State, end: usize, data: &[u8]) -> io::Result<()> {
        let handle = handle_of(data);
        let Some(link) = state.links.iter().find(|link| link[end] == handle) else {
            return Ok(());
        };
        let head = u16::from_le_bytes([data[0], data[1]]);
        // A host's first packet that is not to be flushed reaches the other
        // host as a first packet; a continuing one as it was.
        let boundary = match (head >> 12) & 0x3 {
            0 => 2,
            other => other,
        };
        let head = link[1 - end] | (boundary << 12) | (head & 0xc000);
        let mut forwarded = vec![hci::ACL];
        forwarded.extend(head.to_le_bytes());
        forwarded.extend(&data[2..]);
        self.send(1 - end, &forwarded)?;
        let mut done = vec![1];
        done.extend(handle.to_le_bytes());
        done.extend(1u16.to_le_bytes());
        self.send(end, &hci::event(hci::COMPLETED_PACKETS, &done))
    }
}

/// The connection handle the first two octets of `parameters` carry.
fn handle_of(parameters: &[u8]) -> u16 {
    match parameters {
        [low, high, ..] => u16::from_le_bytes([*low, *high]) & 0x0fff,
        _ => 0,
    }
}

fn connection_complete(status: u8, handle: u16, peer: Address) -> Vec<u8> {
    let mut parameters = vec![status];
    parameters.extend(handle.to_le_bytes());
    parameters.extend(peer.wire());
    parameters.extend([hci::ACL_LINK, 0]);
    hci::event(hci::CONNECTION_COMPLETE, &parameters)
}

fn disconnection_complete(handle: u16, reason: u8) -> Vec<u8> {
    let [low, high] = handle.to_le_bytes();
    hci::event(hci::DISCONNECTION_COMPLETE, &[SUCCESS, low, high, reason])
}
