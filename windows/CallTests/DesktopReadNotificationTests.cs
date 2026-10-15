using System;
using System.Collections.Generic;
using System.Linq;
using IrisChat;

internal static class DesktopReadNotificationTests
{
    internal static void Run()
    {
        var target = new DesktopNotificationTarget(new string('a', 64), "chat", Guid.NewGuid().ToString("N"), "seen", "message");
        var restored = DesktopNotificationNavigation.Decode(DesktopNotificationNavigation.Encode(target))!;
        Check(restored.MessageId == "seen" && restored.Kind == "message", "Action Center metadata survives restart");
        var records = new[] {
            new DesktopReadNotification("read", "iris-messages", restored),
            new DesktopReadNotification("unread", "iris-messages", restored with { MessageId = "older-unread" }),
            new DesktopReadNotification("other", "iris-messages", restored with { Owner = new string('b', 64) }),
            new DesktopReadNotification("call", "iris-calls", restored with { MessageId = null, Kind = null }),
            new DesktopReadNotification("legacy", "iris-messages", restored with { MessageId = null }),
        };
        var exact = DesktopReadNotification.Read(records, "fixture", target.Owner, new HashSet<string>(), (_, _, messages) => {
            Check(messages.Select(message => message.messageId).SequenceEqual(new[] { "seen", "older-unread" }), "Only this account's message alerts are checked");
            return new ulong[] { 0 };
        });
        Check(exact.Select(item => item.Tag).SequenceEqual(new[] { "read" }), "Exact seen clears while older unread and calls remain");
        var legacy = DesktopReadNotification.Read(records, "fixture", target.Owner, new HashSet<string> { "chat" }, (_, _, _) => Array.Empty<ulong>());
        Check(legacy.Select(item => item.Tag).SequenceEqual(new[] { "legacy" }), "Full-chat fallback applies only to identified legacy message alerts");
        Console.WriteLine("PASS: exact message notification cleanup, account/call scoping and persisted identity");
    }

    private static void Check(bool passed, string message) { if (!passed) throw new Exception(message); }
}
