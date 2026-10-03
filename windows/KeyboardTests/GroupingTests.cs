using System;
using System.Windows;
using System.Windows.Controls;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class GroupingTests
{
    public static void Run()
    {
        var time = (ulong)new DateTimeOffset(DateTime.Today.AddHours(12)).ToUnixTimeSeconds();
        var first = new ChatMessageSnapshot("a", "chat", ChatMessageKind.User, "Same name", "person-a", null,
            "Hello", Array.Empty<MessageAttachmentSnapshot>(), Array.Empty<MessageReactionSnapshot>(),
            Array.Empty<MessageReactor>(), false, time, null, DeliveryState.Received,
            Array.Empty<MessageRecipientDeliverySnapshot>(),
            new MessageDeliveryTraceSnapshot(Array.Empty<string>(), Array.Empty<string>(), Array.Empty<string>(), Array.Empty<string>(), null), null);
        var next = first with { id = "b", createdAtSecs = time + 179 };
        Check(!MessageGrouping.Breaks(first, next, ChatKind.Group), "179 seconds joins");
        Check(MessageGrouping.Breaks(first, next with { createdAtSecs = time + 180 }, ChatKind.Group), "180 seconds separates");
        Check(MessageGrouping.Breaks(first, next with { createdAtSecs = time - 1 }, ChatKind.Group), "Backward timestamps separate");
        Check(!MessageGrouping.Breaks(first, next with { author = "Renamed" }, ChatKind.Group), "Stable author survives name change");
        Check(MessageGrouping.Breaks(first, next with { authorOwnerPubkeyHex = "person-b" }, ChatKind.Group), "Identical names do not merge people");
        Check(MessageGrouping.Breaks(first, next with { isOutgoing = true }, ChatKind.Direct), "Direction separates");
        Check(MessageGrouping.Breaks(first with { reactions = new[] { new MessageReactionSnapshot("👍", 1, false) } }, next, ChatKind.Direct), "Prior reaction separates");
        Check(!MessageGrouping.Breaks(first, next with { reactions = new[] { new MessageReactionSnapshot("👍", 1, false) } }, ChatKind.Direct), "Current reaction closes rather than opens cluster");
        var call = first with { call = new CallHistorySnapshot("call", "incoming", "missed", false, time, null, time, 0) };
        Check(MessageGrouping.Breaks(first, call, ChatKind.Direct) && MessageGrouping.Breaks(call, next, ChatKind.Direct), "Call separates both sides");
        Check(MessageGrouping.Breaks(first, next with { kind = ChatMessageKind.System }, ChatKind.Direct), "System separates");
        var midnight = (ulong)new DateTimeOffset(DateTime.Today).ToUnixTimeSeconds();
        Check(MessageGrouping.Breaks(first with { createdAtSecs = midnight - 1 }, next with { createdAtSecs = midnight }, ChatKind.Direct), "Local midnight separates");
        next = next with { createdAtSecs = time + 1 };
        Check(!MessageGrouping.For(null, first, next, ChatKind.Direct).Footer, "Redundant same-minute metadata hidden");
        Check(MessageGrouping.For(null, first, null, ChatKind.Direct).Footer, "Cluster end has footer");
        Check(MessageGrouping.For(null, first, next with { createdAtSecs = time + 60 }, ChatKind.Direct).Footer, "Minute change retains footer without splitting bubble");
        Check(MessageGrouping.For(null, first, next with { delivery = DeliveryState.Seen }, ChatKind.Direct).Footer, "Status change retains footer");
        foreach (var status in new[] { DeliveryState.Pending, DeliveryState.Queued, DeliveryState.Failed })
            Check(MessageGrouping.For(null, first with { delivery = status }, next with { delivery = status }, ChatKind.Direct).Footer, "Unsent status visible");
        Check(MessageGrouping.For(null, first with { expiresAtSecs = time + 1000 }, next, ChatKind.Direct).Footer, "Disappearing status visible");
        var bubble = new MessageBubble();
        bubble.Bind(first, clusterStart: false, clusterEnd: false, showFooter: false);
        var border = (Border)bubble.FindName("Bubble");
        Check(border.CornerRadius == new CornerRadius(4, 18, 18, 4) && border.Margin == new Thickness(0, 1, 0, 1), "Incoming middle bubble shape and spacing");
        Check(((StackPanel)bubble.FindName("MetaRow")).Visibility == Visibility.Collapsed, "Actual footer hidden");
        bubble.Bind(first with { isOutgoing = true }, clusterStart: true, clusterEnd: false);
        Check(border.CornerRadius == new CornerRadius(18, 18, 4, 18) && border.Margin == new Thickness(0, 6, 0, 1), "Outgoing cluster start shape");
        Console.WriteLine("PASS: message grouping boundaries, stable authors, reactions/calls, independent metadata, actual WPF corners");
    }
    private static void Check(bool value, string context) { if (!value) throw new Exception(context); }
}
