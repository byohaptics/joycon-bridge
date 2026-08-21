# Bridge Design

## Identity

- Bridge version: `0.1.0`
- Bridge API version: `0.1.0`
- Transport: OSC over UDP to Bluetooth HID
- Supported host plugin: `io.github.byohaptics.output.joycon.osc`

## Bridge API

[Joy-Con Bridge API Contract](https://github.com/byohaptics/byo-haptics/blob/main/docs/joycon-bridge-contract.md) is the source of truth for OSC paths, argument types, registration, liveness, timeout behavior, and compatibility. This document covers the Bridge application itself.

## Status And Operation

The GUI shows the left and right Joy-Con connection, Bridge process, and Plugin heartbeat as separate states. Status is conveyed by both shape and color: a filled green `●`, red `×`, and gray `－` encode positive, negative, and not checked states. While the Bridge runs, Joy-Con labels report live `接続` or `未接続` state from the service. While it is stopped, a manual check reports only the snapshot `検出済み` or `未検出`; starting the Bridge invalidates that snapshot so a failed start cannot leave a stale `接続` indication. Its workflow is Joy-Con connection check, Bridge operation, then vibration measurement and optimization. Measurement and optimization are not required before normal Bridge use. Normal action buttons share one color and size; only the stop action uses the danger color. Both configured Joy-Cons must be available when the service starts. A HID read or write failure stops the service instead of leaving a nonfunctional process running; reconnect the controller and start the service again.

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
