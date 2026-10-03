using System;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

internal static class MessageGrouping
{
    public readonly record struct Layout(bool Start, bool End, bool Footer);

    public static bool Breaks(ChatMessageSnapshot previous, ChatMessageSnapshot next, ChatKind kind)
    {
        if (previous.kind != ChatMessageKind.User || next.kind != ChatMessageKind.User
            || previous.call != null || next.call != null || previous.reactions.Length != 0
            || previous.isOutgoing != next.isOutgoing || next.createdAtSecs < previous.createdAtSecs
            || next.createdAtSecs - previous.createdAtSecs >= 180
            || LocalDay(previous.createdAtSecs) != LocalDay(next.createdAtSecs)) return true;
        return kind == ChatKind.Group && !next.isOutgoing
            && Author(previous) != Author(next);
    }

    public static Layout For(ChatMessageSnapshot? previous, ChatMessageSnapshot message,
        ChatMessageSnapshot? next, ChatKind kind)
    {
        var start = previous == null || Breaks(previous, message, kind);
        var end = next == null || Breaks(message, next, kind);
        var footer = end || message.expiresAtSecs != null
            || message.delivery is DeliveryState.Pending or DeliveryState.Queued or DeliveryState.Failed
            || message.createdAtSecs / 60 != next!.createdAtSecs / 60
            || message.delivery != next!.delivery;
        return new Layout(start, end, footer);
    }

    private static string Author(ChatMessageSnapshot message) =>
        string.IsNullOrEmpty(message.authorOwnerPubkeyHex) ? message.author : message.authorOwnerPubkeyHex;
    private static DateTime LocalDay(ulong seconds) =>
        DateTimeOffset.FromUnixTimeSeconds((long)seconds).LocalDateTime.Date;
}
