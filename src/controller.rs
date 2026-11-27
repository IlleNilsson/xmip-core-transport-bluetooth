//! Two controllers on this machine joined by one air, so a test reaches the
//! kernel's L2CAP from both ends with no radio between them.
//!
//! Whoever opens `/dev/vhci` has the kernel make a virtual controller, and
//! is that controller: it answers every HCI command the kernel sends it
//! (`hci`, `air`). So the kernel's Bluetooth stack, `BlueZ`'s daemon
//! `bluetoothd` and every L2CAP socket on them are the real ones, and only
//! the air is Xmip's: a page from one controller reaches the other, and ACL
//! data crosses whole. `BlueZ`'s `btvirt` does as much, from a package of
//! its test tools few machines have; this is Xmip's own, answering what the
//! kernel asks of a BR/EDR controller and no more.
//!
//! What the machine gives it, none of it root's to ask: `/dev/vhci` read
//! and written by the user the test runs as, which a udev rule gives a
//! group; `bluetoothd` on the system bus, powering each controller as it
//! comes (`AutoEnable=true`); and, through it over D-Bus (`busctl`), the
//! second controller made discoverable, since one under the kernel's
//! connection control answers a page only from a device it knows or while
//! discoverable. Where the first two are missing a test says so
//! ([`Controllers::unavailable`]). A far end on the second is [`Paired`].

mod air;
mod hci;
mod paired;

pub use paired::Paired;

use std::fs::OpenOptions;
use std::io::{self, PipeWriter};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use transport::error::{Result, TransportError, classify, protocol_error};
use xcore::lock::Locked;

use crate::address::Address;
use air::{Air, End, State, VHCI};

/// Two virtual controllers and the air between them, carried by a thread
/// each until they are dropped, which removes both.
pub struct Controllers {
    air: Arc<Air>,
    stop: Option<PipeWriter>,
    threads: Vec<JoinHandle<()>>,
}

impl Controllers {
    /// Why this machine stands no controllers, or `None` where it does.
    #[must_use]
    pub fn unavailable() -> Option<String> {
        if let Err(error) = OpenOptions::new().read(true).write(true).open(VHCI) {
            return Some(format!(
                "{VHCI}: {error}; a udev rule gives it to the group of the user a test runs as"
            ));
        }
        let daemon = Command::new("busctl")
            .args(["--system", "status", "org.bluez"])
            .output();
        match daemon {
            Ok(daemon) if daemon.status.success() => None,
            Ok(_) => Some("bluetoothd is not on the system bus".to_string()),
            Err(error) => Some(format!("busctl: {error}")),
        }
    }

    /// Two controllers, both powered and the second answering a page,
    /// within `within`.
    ///
    /// # Errors
    /// Where `/dev/vhci` could not be opened, `bluetoothd` powered
    /// neither in time or refused to make the second discoverable, or the
    /// air stopped.
    pub fn pair(within: Duration) -> Result<Self> {
        let air = Arc::new(Air {
            ends: [End::made()?, End::made()?],
            state: Mutex::default(),
            changed: Condvar::new(),
        });
        let (stopped, stop) = io::pipe().map_err(|error| classify("a pipe", &error))?;
        let mut threads = Vec::new();
        for end in 0..2 {
            let stopped = stopped
                .try_clone()
                .map_err(|error| classify("a pipe", &error))?;
            let air = Arc::clone(&air);
            threads.push(std::thread::spawn(move || air.carry(end, &stopped)));
        }
        let controllers = Self {
            air,
            stop: Some(stop),
            threads,
        };
        let deadline = Instant::now() + within;
        let powered = |state: &State| state.powered == [true; 2] && state.index[1].is_some();
        let index = controllers.until(deadline, "bluetoothd powering both", powered)?;
        adapter(index, "Powered")?;
        adapter(index, "Discoverable")?;
        let paged = |state: &State| state.scan[1] & hci::PAGE_SCAN != 0;
        controllers.until(deadline, "the second answering a page", paged)?;
        Ok(controllers)
    }

    /// The first controller: where a near end connects from.
    #[must_use]
    pub fn near(&self) -> Address {
        self.air.ends[0].address
    }

    /// The second controller, answering a page: where a far end listens.
    #[must_use]
    pub fn far(&self) -> Address {
        self.air.ends[1].address
    }

    /// The second controller's index, once `done` holds of the air, before
    /// `deadline`.
    fn until(&self, deadline: Instant, what: &str, done: impl Fn(&State) -> bool) -> Result<u16> {
        let mut state = self.air.state.locked();
        loop {
            if let Some(failed) = &state.failed {
                return Err(protocol_error(format!("the air stopped: {failed}")));
            }
            if done(&state) {
                return Ok(state.index[1].unwrap_or_default());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(TransportError::retryable(format!(
                    "{what} did not happen in time: powered {:?}, scan {:?}, commands \
                     unknown here {:04x?}",
                    state.powered, state.scan, state.unknown
                )));
            }
            state = self
                .air
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for Controllers {
    fn drop(&mut self) {
        drop(self.stop.take());
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// `property` of the adapter `bluetoothd` keeps for controller `index`
/// set true over D-Bus: answered once the daemon has done it.
fn adapter(index: u16, property: &str) -> Result<()> {
    let path = format!("/org/bluez/hci{index}");
    let said = Command::new("busctl")
        .args(["--system", "set-property", "org.bluez", &path])
        .args(["org.bluez.Adapter1", property, "b", "true"])
        .output()
        .map_err(|error| classify("running busctl", &error))?;
    if said.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&said.stderr);
    Err(protocol_error(format!(
        "busctl: {property} on {path}: {}",
        why.trim()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BluetoothTransport;
    use crate::declaration::PSM;
    use crate::socket::Kernel;
    use transport::{Configured, FarEnd, Transport};

    const BOUND: Duration = Duration::from_secs(20);

    #[test]
    fn a_stream_crosses_the_kernels_l2cap_between_two_controllers() {
        if let Some(why) = Controllers::unavailable() {
            println!("bluetooth: SKIPPED: {why}");
            return;
        }
        let far = Paired::stood(PSM, 3, BOUND).expect("two controllers");
        let (near, device) = (
            far.near(),
            Address::parse(far.address()).expect("its address"),
        );
        let client = BluetoothTransport::on(Arc::new(Kernel::new(device, PSM, Some(near))), 3)
            .timing_out_after(BOUND);
        let bytes: Vec<u8> = (0..=255).cycle().take(64 * 1024 + 7).collect();
        let sent = bytes.clone();
        let sending = std::thread::spawn(move || client.send("", &sent));
        let taken = Box::new(far).take_one().expect("taken");
        sending.join().expect("the sender").expect("sent");
        assert_eq!(taken.bytes, bytes);
        assert_eq!(taken.origin_uri, format!("bt://{near}/3"));
        let (name, value) = transport::arrival_identity::peer_mac(&near);
        assert_eq!(taken.observation(&name), Some(value.as_str()), "the sender");
    }

    #[test]
    fn a_device_no_controller_answers_is_said_not_to_have() {
        if Controllers::unavailable().is_some() {
            return;
        }
        let controllers = Controllers::pair(BOUND).expect("two controllers");
        let absent = Address::new([0x02, 0, 0, 0, 0, 0x99]);
        let link = Kernel::new(absent, PSM, Some(controllers.near()));
        let client = BluetoothTransport::on(Arc::new(link), 3).timing_out_after(BOUND);
        let refused = client.send("", b"nobody").expect_err("no one pages back");
        assert!(
            refused.message.contains("02:00:00:00:00:99"),
            "{}",
            refused.message
        );
    }
}
