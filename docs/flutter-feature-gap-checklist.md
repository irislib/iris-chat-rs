# Flutter Feature Gap Checklist

Compared against `~/src/iris-chat-flutter` on 2026-04-24. Sources used include Flutter widget/unit/integration tests plus the matching implementation files.

## Chat And Composer

- [x] Accept copied Nostr profile links in peer inputs, including `https://chat.iris.to/#npub...`, hash-route `#/npub...`, and `nprofile...` links. Flutter coverage: `test/widget/new_chat_screen_test.dart`.
- [x] Match Flutter message grouping rules: group by sender and day, use a strict one-minute threshold for groups, and allow direct-message bubbles across adjacent minute buckets. Flutter coverage: `test/unit/features/chat/presentation/utils/message_grouping_test.dart`.
- [x] Keep the mobile Enter key from submitting messages, while desktop Enter submits. Flutter coverage: `test/widget/message_input_test.dart`; native coverage: iOS/Android UI tests.
- [x] Keep the timeline pinned to the latest message after submit and when new messages arrive while already near the bottom. Native coverage exists in iOS/Android UI tests.
- [x] Basic hashtree attachment send path through native file pickers.
- [x] Strip raw `nhash.../filename` attachment links from displayed message body and render file chips.
- [x] Long-press/context-menu copy for message text and attachment URLs.
- [x] Selected attachment tray before send, with removable file chips. Flutter coverage: `test/widget/message_input_test.dart`.
- [x] Upload progress label and progress bar for attachment upload. Flutter coverage: `test/widget/message_input_test.dart`.
- [x] Desktop emoji button and inline emoji picker. Flutter coverage: `test/widget/message_input_test.dart`.
- [x] Hover action dock on desktop message bubbles with reply, react, and more. Flutter coverage: `test/widget/chat_message_bubble_test.dart`, `integration_test/message_actions_macos_suite.dart`.
- [x] Reply-to message composer state and reply rendering.
- [x] Emoji reactions, reaction aggregation, and reaction push notification text. Flutter coverage: `test/unit/core/utils/reaction_updates_test.dart`.
- [x] Local message delete action.
- [x] Tappable HTTP/www links inside message text. Flutter coverage: `test/widget/chat_message_bubble_test.dart`.
- [x] Inline image attachment preview, fullscreen image viewer, and Escape-to-close on desktop. Flutter coverage: `test/widget/chat_message_bubble_test.dart`.
- [x] Disappearing-message clock and TTL rendering. Flutter coverage: `test/unit/chat_settings_test.dart`, `test/widget/chat_message_bubble_test.dart`.
- [x] Typing indicators. Flutter coverage: `test/unit/core/utils/typing_rumor_test.dart`, `integration_test/app_chat_e2e_macos_suite.dart`.
- [x] Typing and received/seen preference toggles.
- [x] Read/seen receipts beyond the current pending/sent/received/failed states.

## Chats, Invites, And Groups

- [x] Public chat invite creation, join-chat card, invite QR/share flow, and scan/paste invite acceptance. Flutter coverage: `test/widget/create_invite_screen_test.dart`, `test/widget/scan_invite_screen_test.dart`, `test/integration/chat_flow_test.dart`.
- [x] New-chat screen layout with Join Chat, New Chat, and New Group sections in the Flutter order. Flutter coverage: `test/widget/new_chat_screen_test.dart`.
- [x] Invite response subscription for active invites without relying on fixed fetch windows. Flutter coverage: `test/unit/core/utils/invite_response_subscription_test.dart`.
- [x] Group metadata notices and chat settings notices.
- [x] Group member profile-name presentation parity in all member lists.
- [x] Burst incoming message ordering and visibility parity with Flutter macOS e2e tests.

## Profile, Identity, And Devices

- [x] Full settings screen. Flutter coverage: `test/widget/settings_screen_test.dart`.
- [x] Editable profile metadata and profile picture publishing from settings.
- [x] Profile avatars and image modal for own profile picture.
- [x] Export owner/device secret key flows with confirmation dialogs.
- [x] Device registration panel parity: registered/unregistered linked-device copy, read-only linked-device list, delete confirmation polish.
- [x] Source-code/about rows and app version presentation.
- [x] Delete-all-data danger-zone flow with confirmation details.
- [x] Device label/client label niceties from Flutter helpers.

## Network, Notifications, And Runtime Services

- [x] User-editable Nostr relay settings with add/edit/delete validation. Flutter coverage: `test/unit/core/services/nostr_relay_settings_service_test.dart`, `test/widget/settings_screen_test.dart`.
- [x] Offline queue service and queued message state. Flutter README lists offline support; Flutter model has `MessageStatus.queued`.
- [x] Desktop notifications and notification preference.
- [x] Mobile push platform subscription/runtime delivery via APNS/FCM token registration and notification-server delivery. Flutter coverage: `test/unit/core/services/mobile_push_*`, `integration_test/mobile_push_device_delivery_test.dart`.
- [x] Mobile push core/native sync hooks for relay author filters, decryptable session snapshots, and typing/read/settings push suppression.
- [x] Startup-at-login preference on supported desktop platforms. Flutter coverage: `test/unit/core/services/startup_launch_service_test.dart`.
- [x] Connectivity/offline indicator UI.
- [x] Relay connection status/debug visibility beyond support bundle data.

## Media And Hashtree

- [x] Light Rust hashtree upload integration without pulling in Flutter's `hashtree_ffi`.
- [x] Hashtree download path for opening attachments locally.
- [x] Image proxy settings and URL rewrite controls. Flutter coverage: `test/unit/core/services/imgproxy_service_test.dart`, `test/widget/settings_screen_test.dart`.
- [x] Attachment cache management for downloaded media.
- [x] Multiple attachments per composer send.
