# Observe backgrounding during restoration

`ObservedRestorationHome` wraps the existing ordinary-launch controller and
prearmed Home helper. It tails the two already-owned log captures every 5 ms;
it does not query or modify the phone. Once the controller has validated the
actual new process, Home can fire once after that process emits a foreground,
pending `restore_enqueued` marker. A completion in the same observed batch,
already-backgrounded restoration, wrong process, malformed log or logger failure
stops the attempt. It never retries a Home write.

The scheduling decision is not lifecycle acceptance. Keep the existing final
`evaluate_runtime` checks: actual native restoration at background entry,
135 seconds of continuous suspension, unchanged process/audit token and ordinary
resume. Preserve failed observations. Local arrival times do not establish
transport latency or guarantee that Home will beat restoration on a real phone.

In the private driver, construct the wrapper after capture startup and pass
`wrapper.arm` and `wrapper.fire` as the controller callbacks. Close the wrapper
before the capture, including failures, and retain its `record` alongside the
original controller and capture evidence. Bind the wrapper source and focused
qualification to the exact private attempt. Leave app arguments, environment,
restore behavior and all release criteria unchanged.

Run the focused synthetic contracts with:

```sh
python3 -B -m unittest discover -s scripts -p test_ios_early_background_observer.py
```
