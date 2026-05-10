# Iris Chat 2026.5.11.2

- Dark theme is now pure black across iOS, Android, Linux, and Windows — `#000000` background, `#161616` panel, `#262626` panel-alt. Light theme stays pure white.
- Header chrome fades from the bg color at the top to fully transparent at the bottom (no toolbar tone lift, no hairline divider). Identical fade shape in both themes.
- Chat list cleanup: no more hairline dividers between rows; the header avatar + new-chat button share the 16pt gutter with the row avatars; new-chat button is a translucent glass disc instead of a solid accent circle.
- Composer (iOS) gets 6pt vertical breathing room so it no longer sits flush against the keyboard, and the top bar has 6pt bottom inset so the title isn't adjacent to the offline banner.
- Bigger tap targets on the scroll-to-bottom chevron and the send button — off-center thumb taps no longer slip through the composer's transparent gaps to a bubble underneath.
- Opening a long chat lands cleanly at the latest message (Eager VStack timeline, hoisted bottom anchor, double scroll-to-bottom call on initial paint).
- "Message info" is now "Message details" everywhere.
- Android styling tightened to match signal-android: incoming bubble surface variant (#303133 dark / #E7EBF3 light), 20dp avatar→title gap in the chat list row, SemiBold top-bar title.
- Reaction pills (Android + Linux) now use a chat-background-coloured ring so the chip reads as a floating element tucked under the bubble.
