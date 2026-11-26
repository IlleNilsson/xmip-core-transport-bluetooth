//! How a Bluetooth Location is configured: `Configured`, reading the
//! settings its [`declaration`](crate::declaration) declares once
//! (ADR-0064, amendments 2026-09-26 and 2026-11-26).

use std::sync::Arc;

use transport::Configured;
use transport::error::{Result, protocol_error};
use xcore::settings::{Read, Settings};

use crate::{BluetoothTransport, LoopbackRadio, Radio, declaration};

impl Configured for BluetoothTransport {
    const SETTINGS: &'static Settings = declaration::SETTINGS;
    const SCHEMES: &'static [&'static str] = declaration::SCHEMES;

    fn timeout_kept(&mut self) -> Option<&mut dyn transport::configured::TimeoutKept> {
        Some(&mut self.timeout)
    }

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
        Ok(Self::new(radio, channel))
    }
}
