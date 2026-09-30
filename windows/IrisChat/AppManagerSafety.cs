using System;
using System.Linq;
using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    public bool IsRemovedFromGroup(string chatId) =>
        CurrentChat is { kind: ChatKind.Group } chat && chat.chatId == chatId &&
        !(chat.participants?.Any(participant => participant.isLocalOwner) ?? false);

    private bool BlocksRemovedGroupAction(AppAction action)
    {
        var chatId = action switch
        {
            AppAction.SendMessage value => value.chatId,
            AppAction.SendDisappearingMessage value => value.chatId,
            AppAction.SendAttachment value => value.chatId,
            AppAction.SendAttachments value => value.chatId,
            AppAction.SendTyping value => value.chatId,
            AppAction.ToggleReaction value => value.chatId,
            _ => null,
        };
        return chatId != null && IsRemovedFromGroup(chatId);
    }

    public bool IsUserBlocked(string userId)
    {
        var normalized = (userId ?? string.Empty).Trim().ToLowerInvariant();
        return normalized.Length > 0 &&
               (_state.preferences.blockedOwnerPubkeys ?? Array.Empty<string>())
               .Contains(normalized, StringComparer.OrdinalIgnoreCase);
    }

    public void SetUserBlocked(string userId, bool blocked)
    {
        var normalized = (userId ?? string.Empty).Trim().ToLowerInvariant();
        if (normalized.Length == 0) return;
        DispatchToRust(new AppAction.SetUserBlocked(normalized, blocked));
        ShowToast(blocked ? "User blocked" : "User unblocked");
    }

    public void AcceptMessageRequest(string chatId) =>
        DispatchToRust(new AppAction.SetMessageRequestAccepted(chatId));
}
