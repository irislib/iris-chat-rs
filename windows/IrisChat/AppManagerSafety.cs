using System;
using System.Linq;
using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
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
