//! The kernel's L2CAP socket address, `struct sockaddr_l2` of `BlueZ`'s
//! `<bluetooth/l2cap.h>`, which rustix does not write: the one place this
//! crate crosses a C interface (ADR-0050, amendment 2026-09-25;
//! `test/Unsafe.Test.ps1` lists it). Every call on the socket is rustix's
//! own, and safe (`socket`); only the address handed to `bind` and
//! `connect`, and the one `accept` hands back, are laid out here.
#![allow(unsafe_code)]

use std::mem::size_of;

use rustix::net::addr::{SocketAddrArg, SocketAddrLen, SocketAddrOpaque};
use rustix::net::{AddressFamily, SocketAddrAny};

use crate::address::Address;

/// `struct sockaddr_l2`: the family, the PSM little-endian, the device
/// address least significant octet first, the channel identifier, and the
/// address's type, `BDADDR_BREDR`, 0.
#[repr(C)]
#[derive(Clone, Copy)]
struct SockaddrL2 {
    family: u16,
    psm: u16,
    bdaddr: [u8; 6],
    cid: u16,
    bdaddr_type: u8,
}

/// Where an L2CAP socket binds or connects: a device, and a PSM, 0 for
/// whichever the kernel picks.
pub struct L2capAddress(SockaddrL2);

impl L2capAddress {
    pub fn new(device: Address, psm: u16) -> Self {
        Self(SockaddrL2 {
            family: AddressFamily::BLUETOOTH.as_raw(),
            psm: psm.to_le(),
            bdaddr: device.wire(),
            cid: 0,
            bdaddr_type: 0,
        })
    }
}

// SAFETY: `f` is called with a pointer to the `sockaddr_l2` this value holds
// and its size. The value is borrowed for the whole call, so the pointer is
// valid for reads of that many bytes until `f` returns, and nothing writes
// through it; the struct is laid out as the kernel's, `repr(C)`, each field
// the kernel's type: `sa_family_t`, `unsigned short`, `bdaddr_t`,
// `unsigned short`, `uint8_t`.
unsafe impl SocketAddrArg for L2capAddress {
    unsafe fn with_sockaddr<R>(
        &self,
        f: impl FnOnce(*const SocketAddrOpaque, SocketAddrLen) -> R,
    ) -> R {
        let size = SocketAddrLen::try_from(size_of::<SockaddrL2>()).unwrap_or(SocketAddrLen::MAX);
        f((&raw const self.0).cast(), size)
    }
}

/// The device `address` names, the `sockaddr_l2` the kernel handed back
/// with a channel it accepted; `None` where it is no L2CAP address.
pub fn device_of(address: &SocketAddrAny) -> Option<Address> {
    let whole = usize::try_from(address.addr_len()).ok()? >= size_of::<SockaddrL2>();
    if address.address_family() != AddressFamily::BLUETOOTH || !whole {
        return None;
    }
    // SAFETY: `as_ptr` points into the storage `address` owns, borrowed for
    // this block, which holds `addr_len()` bytes the kernel wrote: at least a
    // `sockaddr_l2`, as checked above. `read_unaligned` copies them out
    // without asking the storage for a `sockaddr_l2`'s alignment, and any
    // bytes are a valid `SockaddrL2`, every field an integer.
    let l2 = unsafe { address.as_ptr().cast::<SockaddrL2>().read_unaligned() };
    Some(Address::from_wire(l2.bdaddr))
}
