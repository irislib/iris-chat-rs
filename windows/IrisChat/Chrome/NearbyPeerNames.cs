using System;
using System.Linq;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

public static class NearbyPeerNames
{
    public static DesktopNearbyPeerSnapshot[] Sort(
        AppManager manager,
        DesktopNearbyPeerSnapshot[] peers) =>
        peers
            .OrderBy(peer => HasDirectChat(manager, peer) ? 0 : 1)
            .ThenBy(DeterministicKey, StringComparer.Ordinal)
            .ThenBy(peer => peer.id, StringComparer.Ordinal)
            .ToArray();

    public static string Resolve(
        AppManager manager,
        DesktopNearbyPeerSnapshot peer,
        string fallback = "Nearby user")
    {
        var owner = string.IsNullOrWhiteSpace(peer.ownerPubkeyHex)
            ? null
            : peer.ownerPubkeyHex!.Trim();
        if (owner is not null)
        {
            var chat = manager.ChatList.FirstOrDefault(chat =>
                chat.kind == ChatKind.Direct &&
                string.Equals(chat.chatId, owner, StringComparison.OrdinalIgnoreCase));
            if (!string.IsNullOrWhiteSpace(chat?.displayName))
                return chat!.displayName.Trim();
        }

        return string.IsNullOrWhiteSpace(peer.name) ? fallback : peer.name.Trim();
    }

    public static string Short(string name)
    {
        var trimmed = string.IsNullOrWhiteSpace(name) ? "Nearby" : name.Trim();
        return trimmed.Length <= 14 ? trimmed : trimmed[..13] + "...";
    }

    private static bool HasDirectChat(AppManager manager, DesktopNearbyPeerSnapshot peer)
    {
        var owner = string.IsNullOrWhiteSpace(peer.ownerPubkeyHex)
            ? null
            : peer.ownerPubkeyHex!.Trim();
        return owner is not null && manager.ChatList.Any(chat =>
            chat.kind == ChatKind.Direct &&
            string.Equals(chat.chatId, owner, StringComparison.OrdinalIgnoreCase));
    }

    private static string DeterministicKey(DesktopNearbyPeerSnapshot peer)
    {
        var owner = string.IsNullOrWhiteSpace(peer.ownerPubkeyHex)
            ? null
            : peer.ownerPubkeyHex!.Trim().ToLowerInvariant();
        return owner ?? $"peer:{peer.id.ToLowerInvariant()}";
    }
}
