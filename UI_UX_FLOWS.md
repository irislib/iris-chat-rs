# UI/UX Flows

This document describes the current end-user flows in Iris Chat as implemented
today across the shared Rust router plus the Android, iOS, and macOS shells.

It is intended to be the baseline for future streamlining work. When a user
journey changes, update this document in the same branch.

## Scope

This covers the user-visible flows driven by the current shared route model:

- `Welcome`
- `CreateAccount`
- `RestoreAccount`
- `AddDevice`
- `ChatList`
- `NewChat`
- `NewGroup`
- `Chat`
- `GroupDetails`
- `DeviceRoster`
- `AwaitingDeviceApproval`
- `DeviceRevoked`
- profile/support surfaces

Primary implementation references:

- `core/src/state.rs`
- `core/src/actions.rs`
- `ios/Sources/Views.swift`
- `android/app/src/main/java/to/iris/chat/ui/navigation/NdrApp.kt`

## Product Model

The app is Rust-first:

- Rust owns route state, app state, and domain behavior.
- Native shells render the current `AppState`, host platform integrations, and
  dispatch user actions back to Rust.
- The same logical journeys exist on Android, iOS, and macOS.

The route inventory is currently:

- `Welcome`
- `CreateAccount`
- `RestoreAccount`
- `AddDevice`
- `ChatList`
- `NewChat`
- `NewGroup`
- `Chat { chat_id }`
- `GroupDetails { group_id }`
- `DeviceRoster`
- `AwaitingDeviceApproval`
- `DeviceRevoked`

## Cross-Platform UX Patterns

These patterns apply to most flows:

- Bootstrapping:
  - the shell starts the Rust core
  - secure credentials are restored if present
  - the user either lands in `Welcome` or in a logged-in route stack
- Busy states:
  - long-running actions disable primary buttons and swap labels to verbs like
    `Creating…`, `Restoring…`, `Linking…`, `Sending…`
- Toasts:
  - short success feedback is surfaced as a toast/snackbar
- Back navigation:
  - Rust owns the logical back stack
  - native shells expose back affordances and forward the stack update
- QR and paste:
  - flows that depend on external identifiers generally support both scan and
    paste/manual entry
- Profile access:
  - once logged in, profile/support/device-management flows hang off the main
    chat list screen

## Platform Differences

The logical flows are shared, but the entry affordances differ slightly:

- Android:
  - uses native Compose screens
  - chat list has a chooser path for `New chat` vs `New group`
  - profile appears as a bottom sheet
- iOS:
  - uses SwiftUI full-screen navigation with a modal profile sheet
  - chat list exposes separate `New chat` and `New group` actions directly
- macOS:
  - uses the same SwiftUI Apple shell layer as iOS, with wider rectangular
    desktop layout
  - QR scanning currently falls back to paste/manual entry rather than a live
    camera scanner

## Flow 1: App Launch And Bootstrap

### Goal

Determine whether the user is:

- a first-time user
- a returning owner device
- a returning linked device

### Current path

1. Native constructs the Rust core.
2. Native restores secure credential material if present.
3. Native dispatches restore actions into Rust.
4. Rust emits the initial authoritative route and state.
5. The user lands in:
   - `Welcome` if no restorable session exists
   - `ChatList` or another logged-in flow if a valid session exists
   - `DeviceRevoked` if the current linked device has been removed

### UX notes

- This path is mostly automatic and has no user decisions until bootstrap
  finishes.
- Android has an explicit splash/bootstrap state.
- iOS/macOS currently show an overlay/loading state over the shared shell.

## Flow 2: Create Owner Account

### Entry points

- `Welcome` chooser -> `Create account`

### User intent

Generate a fresh owner account and enter the app as the primary device.

### Current path

1. User lands on the `Welcome` chooser.
2. User taps `Create account`.
3. Rust pushes the `CreateAccount` route.
4. The shell shows a dedicated create-account screen with a single display-name
   field.
5. User enters a display name and taps `Create account`.
6. Shell dispatches `CreateAccount { name }`.
7. Rust creates owner/device credentials and account state.
8. Rust routes the user into the logged-in chat experience.

### Result

- account exists
- current device has owner signing authority
- user lands in `ChatList`
- profile sheet becomes available from the chat list

### Inputs

- display name

### Validation

- empty name disables the primary action

### UX notes

- This is the fastest path through onboarding.
- The chooser keeps this path visually primary and removes restore/link forms
  from the first screen.

## Flow 3: Restore Owner From `nsec`

### Entry points

- `Welcome` chooser -> `Restore account`

### User intent

Bring an existing owner account onto the current device.

### Current path

1. User lands on the `Welcome` chooser.
2. User taps `Restore account`.
3. Rust pushes the `RestoreAccount` route.
4. The shell shows a dedicated restore screen with a single owner-secret field.
5. User pastes or types an owner `nsec`.
6. User taps `Restore account`.
7. Shell dispatches `RestoreSession { owner_nsec }`.
8. Rust reconstructs account state and chat state from the owner credential.
9. User lands in `ChatList`.

### Result

- current device becomes an owner-capable device
- chat list and profile/device-management flows become available

### Inputs

- owner `nsec`

### Validation

- empty input disables the action
- invalid input is handled by Rust; UI currently keeps this flow fairly simple

### UX notes

- This is the highest-trust onboarding path because it imports full owner
  authority.
- The dedicated screen keeps the restore flow separate from linked-device
  onboarding instead of hiding both behind a generic login affordance.

## Flow 4: Start Linked Device

### Entry points

- `Welcome` chooser -> `Add this device`

### User intent

Turn the current device into a secondary/linked device under an existing owner.

### Current path

1. User lands on the `Welcome` chooser.
2. User taps `Add this device`.
3. Rust pushes the `AddDevice` route.
4. The shell shows a dedicated add-device screen with:
   - owner input
   - scan/paste shortcuts
   - a persistent QR panel placeholder
5. User acquires the owner identity by:
   - scanning the owner QR
   - pasting the owner `npub` or hex key
6. User taps the continue action.
7. Shell dispatches `StartLinkedDevice { owner_input }`.
8. Rust creates a local linked-device identity and routes the user to
   `AwaitingDeviceApproval`.

### Result

- current device now has a device identity
- user is blocked pending approval from an owner-capable device
- the placeholder QR panel is replaced by the real approval QR/code

### Inputs

- owner `npub` or owner public key hex

### Validation

- invalid owner input blocks the action
- scan/paste input is normalized before dispatch

### UX notes

- The add-device screen keeps the linking context visible before and after the
  session starts by preserving the QR panel area.
- The actual approval still happens on the owner device in `DeviceRoster`.

## Flow 5: Finish Linked Device Approval

### Entry points

- automatic route after `StartLinkedDevice`

### User intent

Show the information the owner device needs in order to approve the pending
linked device.

### Current path

1. After `StartLinkedDevice`, Rust persists the linked-device session and lands
   on `AwaitingDeviceApproval`.
2. The shell renders this as the waiting state of the same add-device journey.
2. Screen shows:
   - owner `npub`
   - current device `npub`
   - encoded device-approval QR
3. On relaunch, the device comes back to this waiting state rather than the
   initial chooser.
4. User opens the owner device and navigates to `Manage devices`.
5. Owner either scans or pastes the device code into the roster flow.

### Result

- this screen remains the waiting room until the owner approves the device
- once authorized, the linked device converges into the normal logged-in shell

### UX notes

- The screen is strong as a handoff surface because it focuses on one job.
- The user still has to understand the two-device choreography:
  secondary device waits here while the owner device does the approval work.

## Flow 6: Approve Or Remove Devices On The Owner

### Entry points

- `ChatList` -> profile -> `Manage devices`
- direct route push from the profile surface

### User intent

Inspect the owner roster, approve pending devices, and remove linked devices.

### Current path

1. Logged-in user opens profile from `ChatList`.
2. User taps `Manage devices`.
3. App routes to `DeviceRoster`.
4. Screen shows:
   - owner identity
   - current device identity
   - add/approve controls
   - full device list
5. Owner can approve a device by:
   - scanning a QR
   - pasting a device `npub`, hex key, or approval code
   - approving a pending row directly from the roster
6. Owner can remove a non-current linked device from the roster.

### Result

- approved devices transition from pending to authorized
- removed devices are revoked

### Inputs

- device `npub`
- device public key hex
- encoded approval QR/code

### Validation

- approval input is validated against the current owner
- the action is disabled if the current device does not have management rights

### UX notes

- This is the owner control center for device security.
- There are currently two approval affordances:
  - top-level authorize input/button
  - inline `Approve` on pending device rows
- That is useful, but it may be a future streamlining target if we want a
  single clearer primary path.

## Flow 7: Handle Revoked Device

### Entry points

- automatic route when Rust marks the current device as revoked

### User intent

Acknowledge that this linked device is no longer authorized.

### Current path

1. User lands on `DeviceRevoked`.
2. Screen explains that the device was removed from the roster.
3. User taps `Acknowledge`.
4. Shell dispatches `AcknowledgeRevokedDevice`.
5. App returns to a fresh shell.

### Result

- revoked device is no longer treated as logged in
- user is sent back into a clean onboarding state

### UX notes

- This is intentionally narrow and destructive.
- It prevents the shell from pretending the session is still valid.

## Flow 8: Chat List / Home

### Entry points

- automatic landing after successful login/create/restore
- back navigation target from deeper routes

### User intent

View all active conversations and branch into profile, direct chat, or group
creation.

### Current path

1. User lands in `ChatList`.
2. Screen shows:
   - profile entry
   - create conversation actions
   - current account summary
   - existing conversation list
   - empty-state prompt when there are no chats
3. User can:
   - open profile
   - start a direct chat
   - start a group
   - reopen any existing chat thread

### Result

- this is the main hub after onboarding

### Platform note

- Android currently uses a chooser path for new conversation type.
- iOS/macOS expose direct `New chat` and `New group` actions.

### UX notes

- The route is intentionally simple and functions as the main “home”.
- The current model keeps direct chats and groups in one unified list.

## Flow 9: Create Direct Chat

### Entry points

- `ChatList` -> `NewChat`

### User intent

Start or open a one-to-one conversation with another user.

### Current path

1. User enters `NewChat`.
2. User pastes/types/scans a peer identity.
3. User taps `Open chat`.
4. Shell dispatches `CreateChat { peer_input }`.
5. Rust resolves/creates the direct thread and routes to `Chat`.

### Result

- direct thread exists in chat list
- user lands inside the thread

### Inputs

- peer `npub`
- peer public key hex
- `nostr:`-prefixed link

### Validation

- invalid peer input blocks the primary action

### UX notes

- This flow is deliberately low-ceremony.
- The app normalizes `nostr:` links automatically.

## Flow 10: Read And Send Messages In A Chat

### Entry points

- `ChatList` -> existing chat
- `CreateChat`
- `CreateGroup`

### User intent

Read history, send messages, and for groups, jump into group details.

### Current path

1. User enters `Chat`.
2. Screen shows:
   - thread title in top chrome
   - timeline with day chips and bubble styling
   - composer pinned at the bottom
   - for groups, a group-details button in top chrome
3. User types into the composer and taps send.
4. Shell dispatches `SendMessage { chat_id, text }`.
5. Rust updates local thread state and delivery state.

### Result

- new outgoing message appears in the timeline
- delivery state updates from pending toward sent/received/failed

### UX notes

- Chat now opens at the bottom/latest message.
- If the user is already near the bottom, new messages keep the thread pinned
  to the latest message.
- If the user has scrolled up, the app shows a jump-to-bottom affordance.

## Flow 11: Create Group

### Entry points

- `ChatList` -> `NewGroup`

### User intent

Create a named multi-person group conversation.

### Current path

1. User enters `NewGroup`.
2. User enters a group name.
3. User adds members by:
   - pasting/scanning a member identity
   - selecting from existing direct chats
4. Selected members appear as chips.
5. User taps `Create group`.
6. Shell dispatches `CreateGroup { name, member_inputs }`.
7. Rust creates the group thread and routes to `Chat`.

### Result

- new group exists
- user lands in the group thread
- group details become available from the chat chrome

### Inputs

- group name
- member identities

### Validation

- group creation requires:
  - non-empty name
  - at least one selected member

### UX notes

- This is one of the denser flows in the app.
- It currently combines:
  - group metadata
  - direct member entry
  - existing-chat selection
- That is powerful, but likely a future simplification candidate.

## Flow 12: Group Details And Membership Management

### Entry points

- group chat top-bar action -> `GroupDetails`

### User intent

Inspect group membership and, when allowed, manage group metadata.

### Current path

1. User opens group details from a group chat.
2. Screen shows:
   - group name
   - creator and revision metadata
   - member list
3. If the current owner can manage the group, they can:
   - rename the group
   - add members
   - remove non-local members

### Result

- group metadata and membership update in-place
- user stays within the group-management context

### Inputs

- new group name
- added member identity

### UX notes

- This is a secondary management surface, not part of primary onboarding.
- It is discoverable from the chat thread but not directly from the chat list.

## Flow 13: Profile, Identity, And Support

### Entry points

- `ChatList` -> profile affordance

### User intent

Inspect account identity, copy/share keys, manage devices, and access support
tools.

### Current path

1. User opens profile from `ChatList`.
2. Profile shows:
   - display name / owner identity
   - owner QR
   - current device identity
   - device-management entry
   - build / relay-set metadata
   - support bundle actions
   - reset/logout actions
3. User can:
   - copy owner/device identifiers
   - open `DeviceRoster`
   - share or copy the support bundle
   - reset app state
   - logout

### Result

- profile acts as the “account and diagnostics” surface

### UX notes

- This surface mixes:
  - account identity
  - device management
  - support/debug
  - destructive session actions
- It works, but it is one of the clearest candidates for future information
  hierarchy cleanup.

## Flow 14: Support Bundle Export

### Entry points

- profile -> support section

### User intent

Capture current debug/build/support information for troubleshooting.

### Current path

1. User opens profile.
2. User chooses:
   - `Share support bundle`
   - `Copy support bundle`
3. Shell calls the Rust support-bundle export API.
4. Native either:
   - opens the system share surface
   - copies the JSON bundle to the clipboard

### Result

- support/debug payload is available outside the app

### UX notes

- This is intentionally tucked under profile rather than the main chat flows.

## Current Flow Inventory By User Goal

### Become a user

- create owner account
- restore owner session
- start linked device

### Secure and manage the account

- approve linked device
- remove linked device
- acknowledge revoked device
- inspect owner/device identity in profile

### Start conversations

- create direct chat
- create group

### Manage ongoing conversations

- send message
- rename group
- add members
- remove members

### Support and diagnostics

- copy/share support bundle
- reset app state
- logout

## Current Streamlining Hotspots

These are not redesign decisions yet. They are simply the current highest-signal
candidates based on the implemented flows.

### Welcome screen choice density

The welcome flow is now cleaner because it is a chooser instead of a three-form
control panel, but it still asks the user to understand three account models
immediately:

- create account
- restore account
- add this device

That is likely the correct structure, but the copy and hierarchy still matter a
lot.

### Two-device linking choreography

Linking is split across:

- `AddDevice` on the new device
- `AwaitingDeviceApproval` on the new device
- `DeviceRoster` on the owner device

This is functionally sound but mentally heavier than the chat flows.

### Group creation packs in multiple concepts

The current group-create flow combines:

- naming the group
- manually adding member keys
- selecting from existing direct chats

That may be more than the first version needs on one screen.

### Profile mixes multiple information layers

The current profile surface contains:

- identity
- device management
- diagnostics
- destructive actions

Those are related, but not equally important.

### Platform-specific new conversation entry

Android currently uses a chooser path, while iOS/macOS expose separate direct
actions. The logical flow is the same, but the entry interaction differs.

## Recommended Use Of This Doc

When proposing a UX change, describe it against this document in one of these
ways:

- simplify an existing flow
- merge two steps
- split a dense screen into two narrower steps
- move an action to a better parent surface
- reduce platform-specific differences in entry affordances

That will make streamlining proposals easier to review against the current
implemented behavior.
