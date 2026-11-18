# Two-Mac Note to Self acceptance driver

This is an offline source port of the seven-file driver from `2e7b6140ce3deba9878c45a7333f12a1dd0d7a99` onto accepted baseline `e1c30adbcf15af67675bd476c24efb713430a68b`. It preserves the ready-sibling, delayed-copy and accepted-file checks. It does not include the original branch's separate workflow preflight experiment.

The November 5 coverage checkpoint continues to identify the original driver commit and its 19 Python plus 6 simulated UI checks. That historical evidence remains unchanged. The original product was source `1ad1c130ad45df513cd0711f75e1877797fb1213`, run `41814162768`, artifact `13314283543`. Those identifiers are historical evidence, with no default or fallback to that product in the port.

## Offline qualification

Run from the repository with Python 3.12 or later and Node. These checks use disposable synthetic files, SQLite databases, subprocesses and simulated controls. They do not access an account, network, GUI or physical device.

```sh
PYTHONDONTWRITEBYTECODE=1 python3.13 -Werror scripts/note-to-self-macos-runtime/test_runtime.py -v
node scripts/note-to-self-macos-runtime/test_ui.cjs
node --check scripts/note-to-self-macos-runtime/mac_ui.js
git diff --check
```

The storage witnesses run against SQLite definitions read from this repository. The port was reviewed against the CLI device-approval operations, protocol checkpoint core fragment, file destination layout, file-backed synthetic account store, and macOS Accessibility identifiers in the accepted baseline. The notification override uses the current `IRIS_DISABLE_NOTIFICATIONS` interface.

The offline tests reject mismatched run, repository, workflow, artifact, source, tree, checksums and product kind. They also check duplicate offers, file corruption, wrong resumed-device identity, isolation, cleanup and simulated native-control order. They do not establish native Accessibility behavior, actual device linking, network transfer, or two-Mac delivery. Live GUI and physical-device testing remain on their existing holds. This port creates no product or live runtime evidence.

## Future live prerequisites

Only after separate authorization, assign two independent arm64 macOS instances with SSH access, Python 3.12 or later, and disposable graphical console logins expressly reserved for this test. The SSH user must be the reserved graphical user, have Automation and Accessibility access to System Events, and be able to launch the development product. Both instances need access to ordinary device approval, messaging and file-transfer network paths. Use `--screenshots` only with Screen Recording permission for the receiver's test session.

Have the product owner supply the reviewed full source commit, exact successful producer run ID, and exact immutable artifact ID. Pass all three through `--product-source`, `--product-run` and `--product-artifact`. The accepted producer is `ci.yml` or `apple-validation-products.yml` in the established repository. Its run and artifact must name the requested source as their head. A directly dispatched workflow whose head differs from its build input is rejected. The artifact must be named `macos-validation-<source>` and contain the current unsigned arm64 Debug app and CLI receipt. Selecting these inputs does not authorize a build or device use.

On each assigned instance, stage the original artifact ZIP from that exact GitHub artifact ID in a private folder. The coordinator needs read access to GitHub Actions metadata. Each worker verifies the outer ZIP digest, inner archive digest, source tree, architecture, configuration, and development-product fields before extracting. Both workers must report identical app and CLI executable digests. Supply the hosts' configured disk reserve using the larger floor. Staged archives remain the staging owner's responsibility.

Invoke `run.py` with the three product arguments, SSH aliases, reserved usernames, absolute staged ZIP paths, positive `--reserve-gib`, a new private `--receipt` path and `--reserved-test-desktops`. See `run.py --help`. SSH uses known host keys and disables agent forwarding. The coordinator sends the reviewed worker source over SSH and installs no permanent tool. Workers create and remove only their own private temporary directories and manage only their own processes. Account secrets remain within the fresh profile on its originating OS.

## Live acceptance contract

The product CLI creates a disposable primary account and links the sender. A third CLI actor on the receiver instance creates a link request and pauses before approval. Only one-time link inputs cross the private SSH channel. Each Mac then opens its own synthetic profile in the supplied app. The third actor stays paused until the receiver accepts the file.

The driver opens Note to Self, selects **Send directly**, chooses a unique file through the native picker and presses Send. It requires one incoming offer card. Before acceptance, the sender must have a durable copy completed for the receiver and a missing copy for the third device. The receiver chooses a fresh destination through the native folder picker. It must contain exactly one saved file with the source SHA-256 and length. After the third device resumes, all three message IDs, offer content IDs, transfer IDs and manifests must agree. Repeated reads must find one offer per self-chat, and both Mac apps must remain open.

The private receipt records the explicit product identity, worker-source hashes, file and copy checks, and success only after cleanup. Optional screenshots capture only the synthetic receiver window. A VM run must be labeled accordingly and remains separate from verification on the original physical laptop and receiving Mac.
