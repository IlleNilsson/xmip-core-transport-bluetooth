# xmip-core-transport-bluetooth

Bluetooth transport: RFCOMM over L2CAP — a Stream is the information of one data link connection, opened with SABM, carried in UIH frames and closed with DISC; a controller through the kernel's L2CAP sockets on Linux, on a dynamic PSM, or a loopback radio in process. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A send target is read by `net::Target` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one reading of a URI every technology calls: scheme, authority, path and decoded query. Until 2026-09-28 it was read through the transport capability's `socket::target`, which split it on its first slash and left the query in the path.

## A controller

A Location's address names a device: `bt://70:a8:d3:36:de:ad`, the peer a Send Location connects to, or the controller of this machine a Receive Location listens on, `bt://00:00:00:00:00:00` for any. `bt://loopback` is the radio in process. Its settings are `channel`, the RFCOMM server channel; `psm`, the L2CAP PSM, 4097 (0x1001) where it is left out; `controller`, the device address of this machine's controller a Send Location connects from, the kernel's choice where it is left out; and `timeout`.

On Linux a controller is reached through the kernel's L2CAP sockets (`src/socket.rs`), with no privilege: a `SOCK_SEQPACKET` socket, each SDU one RFCOMM frame handed whole, on a dynamic PSM, which a process binds without `CAP_NET_BIND_SERVICE`. The kernel pages the peer, opens the ACL link and the channel, and segments it; RFCOMM above it is Xmip's own (`src/session.rs`), as on the radio in process. RFCOMM's own PSM, 3, is the kernel's RFCOMM's, so a peer reaching this transport is one speaking Xmip's RFCOMM on the PSM its Location names (open problem 229). A Receive Location's listener is bound as its transport is opened (`Configured::listening`), and its receive waits on it. Every call on the socket is rustix's own; the address it takes, `struct sockaddr_l2`, is the crate's one file of unsafe code (`src/system.rs`; ADR-0050, amendment 2026-09-25). On Windows and macOS a device address is refused as the Location is opened, saying so, and `xmip-service` links the transport on Linux alone (ADR-0015, amendment 2026-11-27; open problem 230).

Over `bt://loopback`, and over any `Radio` handed in, the L2CAP is Xmip's own too (`src/l2cap.rs`): the basic frame and the signaling that opens and closes a channel to PSM 3, over an ACL link.

## Two controllers on one machine

A test stands both ends on one Linux machine (`src/controller.rs`, behind `test-support`): the kernel makes a virtual controller for whoever opens `/dev/vhci`, and that one answers every HCI command the kernel sends it. This crate opens two and is both, answering what the kernel asks of a BR/EDR controller with no Secure Simple Pairing and no LE, and carrying pages and ACL data between them; the kernel's stack, BlueZ's `bluetoothd` and every L2CAP socket on them are the real ones. BlueZ's `btvirt` does as much, from its test tools, which few machines have. The second controller is made discoverable through `bluetoothd` over D-Bus (`busctl`), as a controller under the kernel's connection control answers a page only from a device it knows or while discoverable; the far end listens there (`controller::Paired`), and a near end connects from the first.

What a machine gives it, none of it root's at run time:

- the `hci_vhci` module loaded, and `/dev/vhci` read and written by the user the tests run as — on the Linux test computer, `/etc/modules-load.d/xmip-radio.conf` names the module and `/etc/udev/rules.d/70-xmip-vhci.rules` holds `KERNEL=="vhci", GROUP="<user>", MODE="0660"`;
- `bluetoothd` on the system bus, powering a controller as it comes (`AutoEnable=true` in `/etc/bluetooth/main.conf`).

Where either is missing the tests say SKIPPED, and why. The crate's tests carry a Stream of 64 KiB across the kernel's L2CAP between the two, and hold a page no controller answers refused, naming the device; the estate's service test carries a file through a node to the far end on the second (`.src/test/service/far_ends/controller.rs`). No real radio and no independent RFCOMM stack is driven (open problem 229).

## Acknowledgement

Acceptance is at-most-once here. RFCOMM carries data in UIH frames that
nothing acknowledges above the radio's own link, and the client's DISC is
answered as it closes the link: nobody is left to tell how the receive cycle
ended. Each data link's Stream arrives whole.

## The declaration

What a Location of this technology is configured with is two `const`s in its `declaration` module, `src/declaration.rs`, beside the code, which `Configured` reads (ADR-0064, amendment 2026-11-26): `declaration::SETTINGS`, `channel`, `psm`, `controller` and `timeout`, and `SCHEMES`, `bt`. The wait `timeout` defaults to, `TIMEOUT`, and the PSM `psm` defaults to, `PSM`, are there too, and the code reads them from there. It is the same wherever it is built, a box with no Bluetooth controller included. Built with its `declaration` feature alone (`--no-default-features --features declaration`), the crate is those and nothing more: its code and every crate the code is built on are behind the default feature. So the library the surfaces load carries it without the code, through the gathering crate xmip-core-declaration (open problem 48), and the language server and the desktop suggest and check a Bluetooth Location's settings though no node in their process loaded the technology. `test/Declaration.Test.ps1` in the estate holds that the declaration builds nothing the transport capability does not.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
