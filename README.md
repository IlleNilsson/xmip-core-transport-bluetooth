# xmip-core-transport-bluetooth

Bluetooth transport: RFCOMM over L2CAP — a Stream is the information of one data link connection, opened with SABM, carried in UIH frames and closed with DISC; a loopback radio stands in for the controller. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A send target is read by `net::Target` in [xmip-core-library-net](https://github.com/IlleNilsson/xmip-core-library-net), the one reading of a URI every technology calls: scheme, authority, path and decoded query. Until 2026-09-28 it was read through the transport capability's `socket::target`, which split it on its first slash and left the query in the path.

## Acknowledgement

Acceptance is at-most-once here. RFCOMM carries data in UIH frames that
nothing acknowledges above the radio's own link, and the client's DISC is
answered as it closes the link: nobody is left to tell how the receive cycle
ended. Each data link's Stream arrives whole.

## The declaration

What a Location of this technology is configured with is two `const`s in its `declaration` module, `src/declaration.rs`, beside the code, which `Configured` reads (ADR-0064, amendment 2026-11-26): `declaration::SETTINGS`, `channel` and `timeout`, and `SCHEMES`, `bt`. The wait `timeout` defaults to, `TIMEOUT`, is there too, and the code reads it from there, where it was the crate root's until then. It is the same wherever it is built, a box with no Bluetooth controller included. Built with its `declaration` feature alone (`--no-default-features --features declaration`), the crate is those and nothing more: its code and every crate the code is built on are behind the default feature. So the library the surfaces load carries it without the code, through the gathering crate xmip-core-declaration (open problem 48), and the language server and the desktop suggest and check a Bluetooth Location's settings though no node in their process loaded the technology. `test/Declaration.Test.ps1` in the estate holds that the declaration builds nothing the transport capability does not.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
