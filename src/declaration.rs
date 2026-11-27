//! This technology's declaration: the transport guide's "The declaration",
//! `module/core/capability/transport/doc/adding-a-transport.md`. It is the
//! same wherever it is built, a box with no Bluetooth controller included: a
//! Location's settings are suggested and checked wherever it is designed.

use std::time::Duration;

use xcore::settings::{Applies, Fixed, Kind, Presence, Setting, Settings};

/// How long a peer is waited on when a Location says nothing else.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// The L2CAP PSM Xmip's RFCOMM listens on and connects to through a
/// controller when a Location says none: the first dynamic PSM, 0x1001,
/// which a process binds without `CAP_NET_BIND_SERVICE`. RFCOMM's own, 3,
/// is the kernel's RFCOMM's on Linux; the in-process radio speaks it.
pub const PSM: u16 = 0x1001;

/// Every setting a Location takes beyond its address, which names the
/// device — the peer a Send Location connects to, the controller of this
/// machine a Receive Location listens on, `00:00:00:00:00:00` for any —
/// or `loopback`, the in-process radio.
pub const SETTINGS: &Settings = &Settings {
    technology: env!("CARGO_PKG_NAME"),
    settings: &[
        Setting {
            name: "channel",
            kind: Kind::Integer {
                minimum: 1,
                maximum: 30,
            },
            presence: Presence::Required,
            meaning: "The RFCOMM server channel a Send Location opens and a Receive \
                      Location serves.",
            applies: Applies::Both,
        },
        Setting {
            name: "psm",
            kind: Kind::Integer {
                minimum: PSM as i64,
                maximum: 0xffff,
            },
            presence: Presence::Default(Fixed::Integer(PSM as i64)),
            meaning: "The L2CAP PSM a Location on a controller connects to or listens on: \
                      odd, its upper octet even, as a dynamic PSM is.",
            applies: Applies::Both,
        },
        Setting {
            name: "controller",
            kind: Kind::Address,
            presence: Presence::Optional,
            meaning: "The device address of this machine's controller a Send Location \
                      connects from; the kernel's choice where it is left out.",
            applies: Applies::Send,
        },
        transport::configured::timeout(
            Applies::Both,
            Some(TIMEOUT),
            "How long a peer that does not answer is waited on.",
        ),
    ],
};

/// This technology's schemes, as [`transport::Configured::SCHEMES`] defines
/// them.
pub const SCHEMES: &[&str] = &["bt"];
