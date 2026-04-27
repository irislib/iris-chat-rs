# Iris Chat 2.6.30

Test infrastructure fixes that block running the matrix smokes.

- `RealRelayHarnessTest.matchesPeerInput` couldn't find existing direct
  chats by npub since `bdd3695` ("hide raw user ids") stopped exposing
  the peer's npub in the chat-thread `subtitle`. The hex `chatId` never
  matched the npub-form `normalizePeerInput` returned. Add a new
  `peerInputToHex` FFI helper and use it for the comparison so the
  harness recognises existing chats again.
- Two new strict harness methods (`decrypt_notification_payload_from_args`,
  `wait_for_incoming_message_in_open_chat_strict_from_args`) were
  reading args directly via `arguments.getString(...)` but the harness
  base64-encodes everything as `<name>_b64`. Switch to the existing
  `requiredArg` / `optionalArg` helpers so the tests actually receive
  their inputs.
- `direct_chat_live_update_smoke.sh` and `notification_decrypt_e2e.sh`
  now parse `am instrument`'s output for `FAILURES!!!` /
  `INSTRUMENTATION_STATUS_CODE: -[0-9]` markers — `adb shell am
  instrument` returns 0 even when tests fail, so without this the
  smoke printed "passed" when the underlying test failed.

Carries forward the 2.6.29 work: notification decryption with sender +
group name on Android and iOS, App Group / shared-keychain layout for
the iOS Notification Service Extension, bundle id `to.iris.chat`.
