using System;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;

namespace IrisChat;

internal static class ChatMuteMenu
{
    internal static readonly (string Label, ulong Seconds)[] Durations =
    {
        ("1 hour", 3600), ("8 hours", 28800), ("1 day", 86400), ("1 week", 604800),
    };

    internal static MenuItem Create(AppManager manager, string chatId, bool muted)
    {
        var menu = new MenuItem { Header = "Mute notifications" };
        AddItems(menu.Items, manager, chatId, muted);
        return menu;
    }

    internal static void Show(FrameworkElement anchor, AppManager manager, string chatId, bool muted)
    {
        var menu = new ContextMenu { PlacementTarget = anchor, Placement = PlacementMode.Bottom };
        AddItems(menu.Items, manager, chatId, muted);
        menu.IsOpen = true;
    }

    private static void AddItems(ItemCollection items, AppManager manager, string chatId, bool muted)
    {
        if (muted)
        {
            var deadline = manager.Preferences.timedChatMutes.FirstOrDefault(mute => mute.chatId == chatId);
            var status = deadline == null ? "Muted always"
                : $"Muted until {DateTimeOffset.FromUnixTimeSeconds((long)deadline.untilSecs).LocalDateTime:g}";
            items.Add(new MenuItem { Header = status, IsEnabled = false });
            Add("Unmute", () => manager.SetChatMuted(chatId, false));
        }
        foreach (var (label, seconds) in Durations)
            Add(label, () => manager.SetChatMuteUntil(chatId, checked((ulong)DateTimeOffset.UtcNow.ToUnixTimeSeconds() + seconds)));
        Add("Always", () => manager.SetChatMuted(chatId, true));
        void Add(string label, Action action)
        {
            var item = new MenuItem { Header = label };
            item.Click += (_, _) => action();
            items.Add(item);
        }
    }
}
