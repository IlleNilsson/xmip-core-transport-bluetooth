//! How a Bluetooth Location is configured: `Configured`, reading the
//! settings its [`declaration`](crate::declaration) declares once
//! (ADR-0064, amendments 2026-09-26 and 2026-11-26). Its address is
//! `loopback`, the radio in process, or a device address, a controller
//! reached through the kernel's L2CAP sockets, which Linux alone has here.

use std::sync::Arc;

use transport::Configured;
use transport::error::{Result, protocol_error};
use xcore::settings::{Read, Settings};

use crate::{Address, BluetoothTransport, Link, LoopbackRadio, OnRadio, declaration};

impl Configured for BluetoothTransport {
    const SETTINGS: &'static Settings = declaration::SETTINGS;
    const SCHEMES: &'static [&'static str] = declaration::SCHEMES;

    fn timeout_kept(&mut self) -> Option<&mut dyn transport::configured::TimeoutKept> {
        Some(&mut self.timeout)
    }

    fn configured(address: &str, settings: &Read) -> Result<Self> {
        let channel = u8::try_from(settings.integer("channel"))
            .map_err(|_| protocol_error("a server channel over 30"))?;
        let link: Arc<dyn Link> = match address {
            "loopback" => Arc::new(OnRadio(Arc::new(LoopbackRadio::new()))),
            device => {
                let device = Address::parse(device).map_err(|_| {
                    protocol_error(format!(
                        "{device:?} is neither `loopback` nor a device address"
                    ))
                })?;
                let psm = psm(settings.integer("psm"))?;
                let controller = settings.optional_text("controller").map(Address::parse);
                kernel(device, psm, controller.transpose()?)?
            }
        };
        Ok(Self::on(link, channel))
    }

    fn listening(&self) -> Result<Option<&str>> {
        self.link.listening()
    }
}

/// `psm` where it is a dynamic PSM: odd, and its upper octet even.
fn psm(psm: i64) -> Result<u16> {
    u16::try_from(psm)
        .ok()
        .filter(|psm| psm & 0x0001 == 1 && psm & 0x0100 == 0)
        .ok_or_else(|| {
            protocol_error(format!(
                "PSM {psm:#06x} is not one L2CAP gives: odd, and its upper octet even"
            ))
        })
}

/// `device` reached through the kernel's L2CAP sockets on `psm`, from
/// `controller` where one is named.
#[cfg(target_os = "linux")]
#[allow(
    clippy::unnecessary_wraps,
    reason = "a build with no L2CAP socket refuses it"
)]
fn kernel(device: Address, psm: u16, controller: Option<Address>) -> Result<Arc<dyn Link>> {
    Ok(Arc::new(crate::socket::Kernel::new(
        device, psm, controller,
    )))
}

/// Refused: this build reaches no controller.
#[cfg(not(target_os = "linux"))]
fn kernel(device: Address, _: u16, _: Option<Address>) -> Result<Arc<dyn Link>> {
    Err(protocol_error(format!(
        "{device}: a controller is reached through the kernel's L2CAP sockets, which this \
         build has on Linux alone; `loopback` is the radio here"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use transport::Transport;
    use xcore::settings::{Applies, Given};

    fn given(pairs: &[(&str, Given)]) -> Vec<(String, Given)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn a_device_address_is_a_controller_on_a_dynamic_psm() {
        let send = given(&[
            ("channel", Given::Integer(5)),
            ("controller", Given::Text("02:00:00:00:00:01".to_string())),
        ]);
        let built = BluetoothTransport::open("bt://02:00:00:00:00:02", Applies::Send, &send);
        if cfg!(target_os = "linux") {
            let built = built.expect("a controller's Location");
            assert_eq!(built.origin(5), "bt://02:00:00:00:00:02/5");
            assert!(!built.waiting().polled(), "an accept waits");
        } else {
            let refused = built.err().expect("Linux's alone");
            assert!(
                refused.message.contains("Linux alone"),
                "{}",
                refused.message
            );
        }
        for psm in [0x1002, 0x1101, 0x10001] {
            let wrong = given(&[("channel", Given::Integer(5)), ("psm", Given::Integer(psm))]);
            assert!(
                BluetoothTransport::open("bt://02:00:00:00:00:02", Applies::Send, &wrong).is_err(),
                "{psm:#x}"
            );
        }
        let nowhere = given(&[("channel", Given::Integer(5))]);
        let refused = BluetoothTransport::open("bt://nowhere", Applies::Send, &nowhere).err();
        assert!(refused.expect("no device").message.contains("neither"));
        assert_eq!(psm(0x1001).expect("the first"), declaration::PSM);
    }
}
