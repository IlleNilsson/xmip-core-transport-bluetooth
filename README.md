# xmip-core-transport-bluetooth

Bluetooth transport: RFCOMM over L2CAP — a Stream is the information of one data link connection, opened with SABM, carried in UIH frames and closed with DISC; a loopback radio stands in for the controller. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
