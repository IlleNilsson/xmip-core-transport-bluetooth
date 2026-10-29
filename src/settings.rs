//! The settings a Bluetooth Location takes, declared once and read through
//! (ADR-0064, amendment 2026-09-26).

use std::sync::Arc;

use transport::Configured;
use transport::error::{Result, protocol_error};
use xcore::settings::{Applies, Fixed, Kind, Presence, Read, Setting, Settings};

use crate::{BluetoothTransport, LoopbackRadio, Radio, TIMEOUT};

impl Configured for BluetoothTransport {
    /// The address names the radio. `loopback`, the in-process link, is the
    /// one the estate has; a controller over HCI joins when it exposes one.
    const SETTINGS: &'static Settings = &Settings {
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
                name: "timeout",
                kind: Kind::Duration,
                presence: Presence::Default(Fixed::Duration(TIMEOUT)),
                meaning: "How long a peer that does not answer is waited on.",
                applies: Applies::Both,
            },
        ],
    };

    /// The schemes a Location's address is entered with to choose this technology.
    const SCHEMES: &'static [&'static str] = &["bt"];

    fn configured(address: &str, settings: &Read) -> Result<Self> {
        let radio: Arc<dyn Radio> = match address {
            "loopback" => Arc::new(LoopbackRadio::new()),
            other => {
                return Err(protocol_error(format!(
                    "{other:?} is not a radio this estate has; `loopback` is"
                )));
            }
        };
        let channel = u8::try_from(settings.integer("channel"))
            .map_err(|_| protocol_error("a server channel over 30"))?;
        Ok(Self::new(radio, channel).timing_out_after(settings.duration("timeout")))
    }
}
