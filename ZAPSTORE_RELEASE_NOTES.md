# Iris Chat 2.6.28

Push notifications fix.

- Mobile push subscription now reflects the live double-ratchet author set on every state rebuild instead of staying frozen between login and a few `mark_dirty` callsites. Starting a new chat, accepting an invite, or rotating a session immediately tells the notification server which authors to push from. Previously Android often missed notifications until the app was reopened.
- Settings now has a Notifications section with an enable toggle, a custom server URL field (default `notifications.iris.to`), and a link to `github.com/mmalmi/nostr-notification-server`. Same UI on Android and iOS.
- Android FCM e2e test now `force-stop`s the app before pushing the message, so it actually verifies that a closed-app push wakes Iris and surfaces the notification.
