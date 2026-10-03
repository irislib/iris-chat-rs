using System.Linq;
using System.Windows;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

internal static class PersonNames
{
    internal static string? Explicit(string? nickname, string? profileName) =>
        new[] { nickname, profileName }.Select(value => value?.Trim()).FirstOrDefault(value => !string.IsNullOrEmpty(value));

    internal static string? Explicit(string? owner)
    {
        var current = App.CurrentManager.CurrentChat;
        if (current?.kind == ChatKind.Direct && current.chatId == owner)
            return Explicit(current.nickname, current.profileName);
        var chat = App.CurrentManager.ChatList.FirstOrDefault(value => value.kind == ChatKind.Direct && value.chatId == owner);
        return Explicit(chat?.nickname, chat?.profileName);
    }

    internal static PersonNamePresentation Present(string? label, string? identity, string? explicitName = null) =>
        Native.PresentPersonName(label ?? "", identity ?? "", explicitName);

    internal static FontStyle Style(PersonNamePresentation name) => name.isFallback ? FontStyles.Italic : FontStyles.Normal;
}
