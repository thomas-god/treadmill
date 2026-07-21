# `treadmill`

nrf52840-based bridge to transform your Fitness Machine Service (FTMS)
compatible Bluetooth treadmill into a virtual running pod (using Bluetooth
Running Speed and Cadence Service) compatible with sport watches.

### Motivation and architecture

Recent treadmills expose their data via the Bluetooth
[Fitness Machine Service](https://www.bluetooth.com/specifications/specs/fitness-machine-service-1-0/)
to connect to applications like Zwift and Kinomap. But most sport watches are
not compatible with that service and cannot use the treadmill data during an
indoor session. You either have to use the watch inaccurate speed and distance
estimations, or manually set the speed on your watch which can become clunky.

On the other hand the support for the
[Running Speed and Cadence Service (RSC)](https://www.bluetooth.com/specifications/specs/running-speed-and-cadence-service-1-0/)
seems to be more widespread. While RSC is simpler than FTMS, it's enough to get
the treadmill's speed and distance data to your watch and have comparable data
from session to session.

This project uses an nrf52840 micro-controller as the bridge between the
treadmill and your watch as an always-on and low-power solution.

### Compatibility

- Treadmill compatibility: while indications like "Zwift-compatible" are a good
  sign of an FTMS-compatible treadmill, you can use the
  [nRF Connect for mobile app](https://www.nordicsemi.com/Products/Development-tools/nRF-Connect-for-mobile)
  to confirm that. Use the app to connect to your treadmill and check for
  mention of 'Fitness Machine' or 'FTMS'.

- Watch compatibility: varies by manufacturer, refer to their technical
  specifications as its harder to test without a running pod.

### Prerequisites

- An nRF52840 board and a debug probe to flash it,
- [rustup](https://rust-lang.org/tools/install/) and
  [probe-rs](https://probe.rs/docs/getting-started/installation/) installed,
- [Nordic's SoftDevice S140](https://www.nordicsemi.com/Products/Development-software/s140/download),
  version `7.x.x`.

### Installation

1- Install the target toolchain

```bash
rustup target add thumbv7em-none-eabihf
```

2- Erase the chip and flash the SoftDevice (adjust your SoftDevice version)

```bash
probe-rs erase --chip nrf52840_xxAA
probe-rs download --verify --binary-format hex --chip nRF52840_xxAA s140_nrf52_7.X.X_softdevice.hex
```

3- Clone the repository and flash the board

```bash
git clone git@github.com:thomas-god/treadmill.git
cd treadmill
cargo flash --release --chip nRF52840_xxAA
```

Your board will now scan for FTMS-compatible devices, connect to first one and
appear as a Bluetooth device named `RSC service` that your watch should
recognize as a running pod.
