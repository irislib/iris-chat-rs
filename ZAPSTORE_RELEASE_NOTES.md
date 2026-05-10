# Iris Chat 2026.5.10.5

- Tap a reaction pill to see who reacted with what — avatar, username, emoji per person.
- Tap a quoted reply to scroll to the original message.
- Swipe a message to reply (right) or open message info (left). Vertical scrolling stays smooth.
- Quoted-reply preview now stretches to the bubble's full width and tucks reactions under the message instead of as a separate row below.
- Mute the quoted-reply accent rule on incoming bubbles so it reads as a margin marker, not the app accent color.
- Fix iOS bubble width: short messages hug their content, long ones wrap at the row cap instead of stretching.
- Restore left/right alignment for own messages — body text is now left-aligned in own bubbles.
- Nearby chat-list row uses a wireless glyph and a small avatar stack of nearby peers instead of an "N" letter.
- Fix the desktop update banner triggering an "update available" alert on the same version that's already running.
- Faster chat rendering on long conversations: rows skip body re-evaluation when nothing visible changed, the desktop update controller and toast queue are split off so unrelated state updates no longer wake them.
- Batch outgoing delivered receipts during catch-up: a backlog of N messages from one chat sends 1 receipt event with N tags instead of N separate events.
