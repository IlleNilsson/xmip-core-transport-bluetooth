//! This technology's declaration: the transport guide's "The declaration",
//! `module/core/capability/transport/doc/adding-a-transport.md`. It is the
//! same wherever it is built, a box with no Bluetooth controller included: a
//! Location's settings are suggested and checked wherever it is designed.

use std::time::Duration;

use xcore::settings::{Applies, Kind, Presence, Setting, Settings};

/// How long a peer is waited on when a Location says nothing else.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Every setting a Location takes beyond its address, which names the
/// radio: `loopback`, the in-process link, is the one the estate has; a
/// controller over HCI joins when it exposes one.
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
