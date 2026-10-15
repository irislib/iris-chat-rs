using System;
using System.Collections.Generic;
using System.Linq;
using IrisChat.Bindings;

namespace IrisChat;

internal sealed record DesktopReadNotification(string Tag, string Group, DesktopNotificationTarget Target)
{
    internal static IReadOnlyList<DesktopReadNotification> Read(
        IEnumerable<DesktopReadNotification> notifications, string dataDir, string owner,
        IReadOnlySet<string> fullyReadChats,
        Func<string, string, DesktopNotificationMessage[], ulong[]>? lookup = null)
    {
        lookup ??= (directory, account, messages) => Native.ReadDesktopNotificationIndexes(dataDir: directory, ownerPubkeyHex: account, messages: messages);
        var matching = notifications.Where(item => item.Group == "iris-messages" && item.Tag.Length > 0 &&
            item.Target.Kind == "message" && string.Equals(item.Target.Owner, owner, StringComparison.OrdinalIgnoreCase)).ToArray();
        var exact = matching.Where(item => !string.IsNullOrEmpty(item.Target.MessageId)).ToArray();
        var messages = exact.Select(item => new DesktopNotificationMessage(item.Target.Chat, item.Target.MessageId!)).ToArray();
        var read = lookup(dataDir, owner, messages).ToHashSet();
        return exact.Where((_, index) => read.Contains((ulong)index)).Concat(
            matching.Where(item => item.Target.MessageId == null && fullyReadChats.Contains(item.Target.Chat))).ToArray();
    }
}
