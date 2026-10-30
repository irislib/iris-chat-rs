# Ordinary warm lifecycle observation

`ios_warm_lifecycle.py` observes one installed, provenance-bound iOS process.
It imports existing capture, native parsing and ownership checks. Importing it
does not contact a device. It does not measure CPU, replace the app, send APNs,
run XCTest, install a build or change launch arguments or environment.

The release owner must review a hash-bound input and retain the two native
device leases in the observer's direct parent. The existing owned window
enforces a maximum 600-second deadline, bounded output and host disk reserve.
An external owner must also track and close descendant process groups if the
observer is interrupted. Prior evidence and failures remain unchanged.

The sequence activates Settings, resumes the existing Iris process normally,
then observes at least 61 seconds in the foreground. It activates Settings
again, waits for an actual OS suspension event, retains 136 seconds of live
suspended observations, resumes Iris normally, and retains 16 foreground
seconds. The evaluator requires at least 60/135/15 seconds respectively, checks
both device-event and host-observation durations, and rejects every intervening
unexpected state. The initial and final ordinary launch results must have the
same actual PID, executable and audit token as the handed-off launch result.

Every process observation retains its raw CoreDevice inventory. Both raw log
streams, complete prefix hashes, live samples and the intentional shutdown
cutoff are retained. `OstraceCapture` additionally retains selected binary trace
records and reproduces each text record from those bytes during final replay.
The configuration binds the qualified transport and the final record includes
its mandatory `trace_evidence`; missing or failed raw replay cannot pass.
Final replay checks provenance, source qualification,
ordinary operation options, exact operation order, phase boundaries, process
inventories, log health, capture closure and every target state transition.
There is no automatic retry. A failure does not permit changing a requirement
or reusing status flags as evidence.

This result is only the ordinary warm lifecycle observation. Account, message
and draft preservation still require a separate UI followup. Early restoration
overlap, cold background launch and CPU gates remain separate. Qualification
uses synthetic records and does not constitute physical release acceptance.
