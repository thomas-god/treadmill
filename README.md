# `treadmill`

Transform your Bluetooth treadmill into a virtual running pod using an
nRF52840-based board as a bridge. This allows your sport watch to get the
treadmill's speed and distance data during an indoor training session.

### Motivation and architecture

Modern treadmills expose their data via the Bluetooth
[Fitness Machine Service (FTMS)](https://www.bluetooth.com/specifications/specs/fitness-machine-service-1-0/)
to connect to applications like Zwift and Kinomap. But most sport watches are
not compatible with that service and cannot use treadmill data during an indoor
session. You either have to use the watch inaccurate speed and distance
estimations, or manually set the speed on your watch which can become clunky.

On the other hand sport watches support for the Bluetooth
[Running Speed and Cadence Service (RSC)](https://www.bluetooth.com/specifications/specs/running-speed-and-cadence-service-1-0/)
seems to be more widespread. While RSC is simpler than FTMS in term of features
and data supported, it's enough to get the treadmill's speed and distance to
your watch and have comparable training data from session to session.

To provide an _always-on_ (anyone in your household can connect to it without an
additional external application) and _cheap_ (board cost and low power
consumption) solution, this project is based on an nrf52840 micro-controller to
act as the bridge between the treadmill and sport watches.

### Compatibility

- Treadmill compatibility: while indications like "Zwift-compatible" are a good
  sign of an FTMS-compatible treadmill, you can use the
  [nRF Connect for mobile app](https://www.nordicsemi.com/Products/Development-tools/nRF-Connect-for-mobile)
  to confirm that. Use the app to connect to your treadmill and check for
  mention of 'Fitness Machine' or 'FTMS'.

- Watch compatibility: varies by manufacturer, refer to their technical
  specifications as its harder to test without an actual running pod. Be careful
  that ANT+ only compatibility will not work.

### Prerequisites

- An nRF52840 board and a debug probe to flash it,
- [rustup](https://rust-lang.org/tools/install/) and
  [probe-rs](https://probe.rs/docs/getting-started/installation/) installed,
- [Nordic's SoftDevice S140](https://www.nordicsemi.com/Products/Development-software/s140/download),
  version `7.x.x`.

### Installation

1. Install the target toolchain

```bash
rustup target add thumbv7em-none-eabihf
```

2. Erase the chip and flash the SoftDevice (adjust your SoftDevice version
   accordingly)

```bash
probe-rs erase --chip nrf52840_xxAA
```

```bash
probe-rs download --verify --binary-format hex --chip nRF52840_xxAA s140_nrf52_7.X.X_softdevice.hex
```

3. Clone the repository and flash the board

```bash
git clone git@github.com:thomas-god/treadmill.git
cd treadmill/firmware
cargo flash --release --chip nRF52840_xxAA
```

Your board will now scan for FTMS-compatible devices, connect to first one and
appear as a Bluetooth device named `RSC service` that your watch should
recognize as a running pod.

### Limitations

- **No control of the treadmill from the watch**: since RSC is a read-only
  service, you cannot control your treadmill's speed and inclination from the
  watch as you would do from Zwift or when using a bike home trainer. Physical
  control of the treadmill remains possible when connected to it.

- **Fixed cadence**: the RSC expects a cadence value when receiving data from a
  running pod. Since treadmills cannot measure cadence the bridge returns a
  constant placeholder value instead. From our testing sport watches will
  prioritize the speed information when computing other training metrics, but
  your mileage may vary depending on the watch.

- **No inclination or heart rate data**: RSC does not have a notion of
  inclination or heart rate so that:
  - recorded data on your watch will be flat,
  - if you usually connect a hear rate sensor to the treadmill, connect it
    directly to your watch instead.

### Tested devices

#### Boards

- [Seeed Studio XIAO nRF52840](https://www.seeedstudio.com/Seeed-XIAO-BLE-nRF52840-p-5201.html)

#### Treadmills

- Domyos RUN500

#### Sport watches

- Coros APEX Pro 2
- Garmin Instinct Solar 2

### Future features

- Compile time variables to control:
  - the placeholder cadence value,
  - the device name or Bluetooth address to connect to (to handle multiple
    FTMS-compatible devices in the same range/room),
  - name and BLE address of the virtual running pod.
- Loop mode: go back to scanning for FTMS devices when connection ends/fails.
- Simple desktop CLI to emulate the virtual running pod to easily test one's
  watch compatibility.

## Development

Same prerequisites and installation, but use `cargo run` instead of
`cargo flash` to use the debug mode of `probe-rs` to get the board's logs to
your development machine.

### Design decisions

Since the project scope is small (FTMS to RSC translation) and fixed (the
protocols are fixed and won't evolve) we chose not to use a full blown
domain/infrastructure separation as it would add more complexity via indirection
layers without the usual benefits (evolvability, infrastructure/hardware
abstraction).

- `fmts-rsc` contains the hardware agnostic logic (parsing and encoding
  from/into array of bytes, protocol conversion) and lives in a dedicated crate
  to facilitate testing (`test` needs `std` while the rest is `no_std`),
- `firmware` contains the hardware dependant logic and is dependant on
  `ftms-rsc`.
- _Note that we don't use cargo workspace for these two crates to allow
  different target and dependencies per crate (in fact `ftms-rsc` has no
  dependencies)._

Testing:

```bash
cd ftms-rsc
cargo test
```

Flashing and running the board in debug/development mode

```bash
cd firmware
cargo run
```

### Broad architecture

The board will first enters scanning mode and will wait for any FTMS-compatible
treadmill. It will then connect to the first device found and check for the
actual FTMS service and Treadmill Data characteristic. Then a Central (connected
to the treadmill) and a Peripheral (exposing an RSC service and notifying RSC
measurements) will run in parallel, communicating via an
`embassy_sync::pubsub::PubSubChannel`.
