# Bridge Design

## Identity

- Bridge version: `0.1.0`
- Bridge API version: `0.1.0`
- Transport: OSC over UDP to Bluetooth HID
- Supported host plugin: `io.github.byohaptics.output.joycon.osc`

## Bridge API

[Joy-Con Bridge API Contract](https://github.com/byohaptics/byo-haptics/blob/main/docs/joycon-bridge-contract.md) is the source of truth for OSC paths, argument types, registration, liveness, timeout behavior, and compatibility. This document covers the Bridge application itself.

## Status And Operation

A state card leads: whether it is vibrating, the current status line, and the start or stop action. Under it the left Joy-Con, the right Joy-Con, and the Plugin heartbeat are listed as rows. Vibration measurement and the operation log are collapsed until asked for. The display is not numbered as a sequence of steps; it is read at a glance, not walked through once.

Status is encoded by shape and color together, so it survives without color vision: a filled green `●` is active, an amber `○` is pending, a gray `－` is inactive or not yet checked, and a red `×` is a fault. Red carries only genuine faults; a stopped Bridge and a Bridge waiting for its peer are ordinary states.

While the Bridge runs, Joy-Con rows report live `接続` or `接続待ち` from the service. While it is stopped, a manual check reports only the snapshot `検出済み` or `未検出`; starting the Bridge invalidates that snapshot so a failed start cannot leave a stale `接続` indication. One status line lives in the state card and every transition keeps it current.

Filled buttons of one size and color are actions, the stop action alone uses the danger color, and section toggles are outlined. A failure message points at the operation log and opens it. The page scrolls as a whole.

Both configured Joy-Cons must be available when the service starts. A HID read or write failure stops the service instead of leaving a nonfunctional process running; reconnect the controller and start the service again.

The Windows release is one self-contained `Joy-Con Bridge` application. The user is never asked to place, select, or start a second program, enter a file location, edit a communication address, or use a command line. The same application runs both the visible controls and the background vibration service without extracting another program. Measurement data, the optimized profile, and diagnostic details use the current Windows user's local application-data area automatically.

## Output Scheduling

OSC receive processing keeps only the latest requested values. The sensation engine calculates a frame every `50 ms`; HID output refresh runs every `15 ms`. Slow writes therefore do not build an unbounded queue of stale rumble commands. A transition to zero sends an immediate stop report.

## Sensation Mapping

- Force maps value linearly to amplitude and raises requested frequency from `20 Hz` to `160 Hz`.
- Vibration reaches full requested amplitude at value `0.05` and raises requested frequency from `5 Hz` to `320 Hz`.
- Pain uses a pulsed envelope and bounded random variation.
- Simultaneous sensations are blended by their current values.
- Temperature is unsupported in Bridge API `0.1.0`.

Frequency carriers may be replaced by a per-controller IMU profile. Without a profile, built-in carrier bands are used.

## Configuration

Internal configuration stores the listen address, namespace, timeout, frequency scale, profile path, and two controller bindings. Each binding contains side, Bluetooth address selection, OSC Target name, and player ID. The standard application uses automatic selection for the first connected controller of each side and requires no user configuration. Developer-only configuration and command-line overrides remain available for diagnostics and nonstandard multi-controller setups.

Use `--device SIDE,AUTO_OR_BLUETOOTH_ADDRESS,OSC_ADDRESS,ID` to set a binding and `--save-config` to persist effective values.

## Utilities

- `joycon-list`: list paired/openable controllers.
- `joycon-rumble-test`: send a short direct hardware test.
- `joycon-imu-sweep`: measure candidate carriers and save an optimized profile.
- `--dry-run`: parse and route without accessing HID devices.
- `--trace-csv`: record receive, frame, HID, and optional IMU timing for diagnostics.
