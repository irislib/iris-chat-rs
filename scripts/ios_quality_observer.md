# Installed iPhone quality observation

`ios_quality_observer.py` records Activity Monitor cumulative CPU and disk-write
counters, physical-device thermal intervals, live SpringBoard foreground state,
and CoreDevice process inventories for an already installed production app.
This is a separately qualified measurement method. Historical XCTest CPU
receipts retain their original method and do not qualify this observer.

The fixed measurement is 180 seconds of foreground settling, one discarded
60-second interval, then two retained 60-second intervals. Each retained
interval must independently use at most 5% of one CPU core and 5 MiB/minute of
disk writes. Thermal state must be naturally nominal, the process unchanged,
foreground observation gaps at most 2.5 seconds, and process-counter gaps at
most 8 seconds. CPU is the difference in cumulative system plus user CPU time,
divided by measured elapsed time, without core-count normalization or overhead
subtraction. Writes use cumulative `disk-bytes-written` bytes.

Observed boundaries are the first eligible samples, selected before metric
values are read. The unchanged v3 tolerances are 179.9–183 seconds for settling,
59.9–63 seconds for each other observed interval, 57–72 seconds for each metric
interval, and at most two seconds between its metric and observed duration.
UTC and monotonic clocks are paired and checked against the trace's own dates.

## Offline qualification

Use `qualify_ios_quality_observer.py --out NEW_PRIVATE_DIRECTORY
--tagged-validator EXACT_TAGGED_VALIDATOR --counters RETAINED_SYSMON_XML
--toc RETAINED_TOC_XML --thermal RETAINED_THERMAL_XML
--reference RETAINED_CPU_MEASUREMENT_JSON`. Supply the retained Activity Monitor
exports and their independently recorded CPU result. The helper runs the
synthetic failure and complete-driver tests plus existing dependency tests. It
also replays actual-format CPU counters against the retained reference and
checks disk-counter and thermal fields. It writes source, input, interpreter,
and test-log digests. No device is contacted and no native build is run.

This qualifies parsing and control flow, not physical-device acceptance or
observer overhead. It never substitutes the unrelated application's CPU result
for the release candidate. A physical run must produce fresh candidate-bound
evidence; an unsupported trace format or unobserved foreground state fails.

## Release-owner execution

Execution requires the release owner's accepted distribution, exact installed
candidate/PID, and existing `OwnedWindow` configuration. The latter includes
exact-device and acceptance leases, candidate/provisioning provenance, pinned
tools, explicit output/free-space limits, and strict capture closure. Its Home
helper bindings remain required by that existing configuration, but this
observer does not invoke the helper or any HID action.

Create a private JSON input with these fields; each binding is an absolute
`path` and its `sha256`:

```json
{
  "schema": "iris-quality-observer-input-v1",
  "window": {"path": "...", "sha256": "..."},
  "xctrace": {"path": "...", "sha256": "..."},
  "pid": 123,
  "qualification": {"path": "...", "sha256": "..."}
}
```

The existing app should be running in the background before this run. After
live logging and trace readiness, one ordinary CoreDevice activation must
resume that exact PID and emit a real foreground state update. The observer
does not terminate or relaunch it. A changed PID or missing state fails.

Run `ios_quality_observer.py --input PRIVATE_INPUT --input-sha256 INPUT_SHA256
--output NEW_PRIVATE_DIRECTORY --execute` only inside the owner's authorized
quiet device window. Activity Monitor records all processes for 390 seconds;
the 360-second measurement starts after the fixed setup delay and activation.
Inventories and foreground logging continue until exports and closure finish.
The existing window bounds the whole operation to 600 seconds and 100 MiB.
The observer installs nothing, builds nothing, does not attach a debugger or
XCTest runner, and does not retry failures.

Successful output includes raw trace exports, actual process inventories,
paired clocks, exact live-log prefixes, clean closure, `capture.json`, and the
safe v3 `cpu.json` component. Failures preserve `failure.json` and raw evidence.
Startup, sending, crash/lifecycle checks, distribution acceptance, and the
release owner's acceptance remain separate. Private evidence may contain
device and executable identifiers; publish only the safe receipt.
