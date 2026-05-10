# Iris Chat 2026.5.11.1

- Signal-style chat-list cleanup: rows no longer have hairline dividers between them, and the header avatar + new-chat button now share the same 16pt gutter as the chat rows below.
- New-chat button in the chat list is now a translucent glass disc instead of a solid accent circle — matches the composer's glass attach button.
- Dark theme reset to pure black (background, panel, panelAlt sliders all dropped to Signal-iOS's `#000000` / `#161616` / `#262626` values). Light theme stays pure white.
- Header chrome fades from the bg color at the top to fully transparent at the bottom — no toolbar tone lift, no hairline divider. Same fade shape in both themes.
- "Message info" is now "Message details" across iOS, Android, Linux, and Windows.
- Bigger tap targets on the scroll-to-bottom chevron and the send button — off-center thumb taps no longer slip through the composer's transparent gaps to the bubble underneath.
- Composer alignment fixes on iOS: top-bar back button shares an x with the composer's plus button, and the input pill is height-matched to the glass attach/send buttons so `.bottom` alignment reads as centered.
