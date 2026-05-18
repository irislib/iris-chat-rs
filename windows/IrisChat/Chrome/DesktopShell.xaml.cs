using System;
using System.ComponentModel;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using IrisChat.Bindings;
using IrisChat.Views;

namespace IrisChat.Chrome;

public partial class DesktopShell : UserControl
{
    private readonly AppManager _manager;
    private Screen _activeScreen;
    private string? _renderedChatId;
    private string? _renderedScreenKey;
    private bool _showingNearby;
    private bool _syncingSearchText;
    private string _searchQuery = string.Empty;

    public DesktopShell(AppManager manager, Screen activeScreen)
    {
        InitializeComponent();
        _manager = manager;
        _activeScreen = activeScreen;

        Loaded += (_, _) =>
        {
            _manager.PropertyChanged += OnManagerChanged;
            Refresh();
        };
        Unloaded += (_, _) => _manager.PropertyChanged -= OnManagerChanged;
    }

    public void SetActiveScreen(Screen screen)
    {
        _activeScreen = screen;
        Refresh();
    }

    private void OnManagerChanged(object? sender, PropertyChangedEventArgs e)
    {
        // Always pull the freshest active screen from the manager so back-pops
        // and pushes drive the right-pane content even without explicit shell
        // recreation by RootView.
        _activeScreen = _manager.ActiveScreen;
        Refresh();
    }

    private void Refresh()
    {
        // Sidebar profile
        var account = _manager.Account;
        if (account != null)
        {
            ProfileAvatar.Label = string.IsNullOrEmpty(account.displayName)
                ? "Iris user"
                : account.displayName;
            ProfileAvatar.PictureUrl = account.pictureUrl;
        }

        SyncSearchChrome();

        // Sidebar chat list
        var chats = _manager.ChatList ?? Array.Empty<ChatThreadSnapshot>();
        var activeChatId = _activeScreen is Screen.Chat c ? c.chatId : null;
        RefreshChatRows(chats, activeChatId);

        if (_showingNearby)
        {
            if (_renderedScreenKey != "nearby")
            {
                _renderedScreenKey = "nearby";
                MainHost.Content = new NearbyView();
            }
            return;
        }

        // Main pane: keep ChatView mounted across state ticks for the same
        // chat to preserve scroll/composer state.
        var key = RootView.ScreenKey(_activeScreen);
        if (_renderedScreenKey != key)
        {
            _renderedScreenKey = key;
            MainHost.Content = BuildMainPane(_activeScreen);
        }
        else if (_activeScreen is Screen.Chat c2 && _renderedChatId != c2.chatId)
        {
            _renderedChatId = c2.chatId;
        }
    }

    private void RefreshChatRows(ChatThreadSnapshot[] chats, string? activeChatId)
    {
        ChatRows.Items.Clear();
        var query = _searchQuery.Trim();

        if (query.Length == 0 && _manager.Preferences.nearbyEnabled)
        {
            ChatRows.Items.Add(BuildNearbyRow());
        }

        var visibleChats = query.Length == 0
            ? chats
            : chats.Where(chat => ChatMatchesQuery(chat, query)).ToArray();

        foreach (var chat in visibleChats)
        {
            ChatRows.Items.Add(BuildChatRow(chat, activeChatId));
        }

        if (query.Length > 0 && visibleChats.Length == 0)
        {
            ChatRows.Items.Add(BuildSearchEmptyRow());
        }
    }

    private ChatRow BuildChatRow(ChatThreadSnapshot chat, string? activeChatId)
    {
        var row = new ChatRow { Chat = chat, IsActive = chat.chatId == activeChatId };
        row.Activated += chosen =>
        {
            _showingNearby = false;
            ClearSidebarSearch(focus: false, refresh: false);
            _manager.OpenChat(chosen.chatId);
        };
        row.ContextMenu = BuildChatContextMenu(chat);
        return row;
    }

    private FrameworkElement BuildSearchEmptyRow() =>
        new Border
        {
            Padding = new Thickness(12, 28, 12, 28),
            Child = new TextBlock
            {
                Text = "No matches",
                Foreground = (System.Windows.Media.Brush)FindResource("TextMuted"),
                FontSize = 13,
                HorizontalAlignment = HorizontalAlignment.Center,
            },
        };

    private static bool ChatMatchesQuery(ChatThreadSnapshot chat, string query)
    {
        var terms = query.Split(' ', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        return terms.Length == 0 || terms.All(term =>
            ContainsSearch(chat.displayName, term)
            || ContainsSearch(chat.subtitle, term)
            || ContainsSearch(chat.chatId, term)
            || ContainsSearch(chat.lastMessagePreview, term)
            || ContainsSearch(chat.draft, term));
    }

    private static bool ContainsSearch(string? value, string query) =>
        !string.IsNullOrWhiteSpace(value)
        && value.IndexOf(query, StringComparison.OrdinalIgnoreCase) >= 0;

    private FrameworkElement BuildMainPane(Screen screen) => screen switch
    {
        Screen.Chat c => CreateChatPane(c.chatId),
        Screen.GroupDetails g => new GroupDetailsView { GroupId = g.groupId },
        Screen.NewChat => new NewChatView(),
        Screen.NewGroup => new NewGroupView(),
        Screen.CreateInvite => new CreateInviteView(),
        Screen.JoinInvite => new JoinInviteView(),
        Screen.Settings => new SettingsView(),
        Screen.DeviceRoster => new DeviceRosterView(),
        // No chat selected (default route) → land on the new-chat surface so the
        // user always has invite + paste-link affordances visible.
        Screen.ChatList => new NewChatView(),
        _ => new NewChatView(),
    };

    private void SyncSearchChrome()
    {
        var hasQuery = _searchQuery.Length > 0;
        SearchPlaceholder.Visibility = hasQuery ? Visibility.Collapsed : Visibility.Visible;
        ClearSearchButton.Visibility = hasQuery ? Visibility.Visible : Visibility.Collapsed;
        if (SearchInput.Text == _searchQuery) return;

        _syncingSearchText = true;
        try
        {
            SearchInput.Text = _searchQuery;
        }
        finally
        {
            _syncingSearchText = false;
        }
    }

    private void OnSearchTextChanged(object sender, TextChangedEventArgs e)
    {
        if (_syncingSearchText) return;
        _searchQuery = SearchInput.Text ?? string.Empty;
        var trimmed = _searchQuery.Trim();
        if (TryDispatchSearchShortcut(trimmed)) return;
        Refresh();
    }

    private bool TryDispatchSearchShortcut(string trimmed)
    {
        if (trimmed.Length == 0) return false;

        ChatInputShortcut? shortcut;
        try
        {
            shortcut = Native.ClassifyChatInput(trimmed);
        }
        catch
        {
            return false;
        }

        switch (shortcut)
        {
            case ChatInputShortcut.DirectPeer peer:
                _showingNearby = false;
                ClearSidebarSearch(focus: false, refresh: true);
                _manager.CreateChat(peer.peerInput);
                return true;

            case ChatInputShortcut.Invite invite:
                _showingNearby = false;
                ClearSidebarSearch(focus: false, refresh: true);
                _manager.AcceptInvite(invite.inviteInput);
                return true;

            default:
                return false;
        }
    }

    private void OnSearchKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Escape) return;
        ClearSidebarSearch(focus: true, refresh: true);
        e.Handled = true;
    }

    private void OnClearSearch(object sender, RoutedEventArgs e) =>
        ClearSidebarSearch(focus: true, refresh: true);

    private void ClearSidebarSearch(bool focus, bool refresh)
    {
        _searchQuery = string.Empty;
        if (SearchInput.Text.Length > 0)
        {
            _syncingSearchText = true;
            try
            {
                SearchInput.Clear();
            }
            finally
            {
                _syncingSearchText = false;
            }
        }
        SyncSearchChrome();
        if (focus) SearchInput.Focus();
        if (refresh) Refresh();
    }

    private FrameworkElement CreateChatPane(string chatId)
    {
        _renderedChatId = chatId;
        return new ChatView { ChatId = chatId };
    }

    private FrameworkElement BuildNearbyRow()
    {
        var snapshot = _manager.NearbySnapshot;
        var subtitle = !snapshot.visible
            ? "Click to enable"
            : snapshot.peers.Length > 0
                ? NearbySummary(snapshot.peers)
                : WifiStatusLabel(snapshot.status);

        // Match the regular ChatRow layout: 44px wireless badge in the
        // leading slot, 12px gutter, then title + subtitle stack with an
        // inline avatar group sitting next to the "Boromir nearby" label.
        var grid = new Grid { Margin = new Thickness(10, 8, 10, 8) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        var leading = BuildWirelessIcon();
        Grid.SetColumn(leading, 0);

        var text = new StackPanel
        {
            Orientation = Orientation.Vertical,
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(12, 0, 0, 0),
        };
        text.Children.Add(new TextBlock
        {
            Text = "Nearby",
            Foreground = (System.Windows.Media.Brush)FindResource("TextPrimary"),
            FontSize = 15,
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
        });

        var subtitleRow = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Margin = new Thickness(0, 2, 0, 0),
            VerticalAlignment = VerticalAlignment.Center,
            // Pin the height so the row doesn't grow vertically when the
            // avatar stack toggles in/out — the subtitle text alone is
            // ~17px, and the 16px avatars below fit inside that.
            MinHeight = 18,
        };
        if (snapshot.peers.Length > 0)
        {
            subtitleRow.Children.Add(BuildNearbyAvatarStack(snapshot.peers));
        }
        subtitleRow.Children.Add(new TextBlock
        {
            Text = subtitle,
            Foreground = (System.Windows.Media.Brush)FindResource("TextMuted"),
            FontSize = 13,
            Margin = new Thickness(snapshot.peers.Length > 0 ? 6 : 0, 0, 0, 0),
            VerticalAlignment = VerticalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        text.Children.Add(subtitleRow);
        Grid.SetColumn(text, 1);

        grid.Children.Add(leading);
        grid.Children.Add(text);

        var button = new Button
        {
            Style = (Style)FindResource("GhostButton"),
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Padding = new Thickness(0),
            Margin = new Thickness(0, 0, 0, 4),
            Content = grid,
        };
        button.Click += OnNearby;
        return button;
    }

    // 44px round wireless badge that always occupies the leading slot of
    // the Nearby chat-list row, so the row visually lines up with regular
    // chat avatars.
    private FrameworkElement BuildWirelessIcon()
    {
        const double Size = 44;
        return new System.Windows.Controls.Border
        {
            Width = Size,
            Height = Size,
            CornerRadius = new CornerRadius(Size / 2),
            Background = (System.Windows.Media.Brush)FindResource("Panel"),
            VerticalAlignment = VerticalAlignment.Center,
            Child = new TextBlock
            {
                // Segoe MDL2 Assets E701 = wireless / wifi fan
                Text = "\uE701",
                FontFamily = new System.Windows.Media.FontFamily("Segoe MDL2 Assets"),
                FontSize = 20,
                Foreground = (System.Windows.Media.Brush)FindResource("TextPrimary"),
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
            },
        };
    }

    // Small inline avatar group rendered next to the subtitle text, so
    // when peers are around their faces show up alongside their names
    // ("Boromir nearby"). Up to three avatars overlap by ~6px each.
    private static FrameworkElement BuildNearbyAvatarStack(DesktopNearbyPeerSnapshot[] peers)
    {
        const double AvatarSize = 16;
        const double Overlap = 6;
        var stride = AvatarSize - Overlap;
        var take = Math.Min(peers.Length, 3);
        var stackWidth = (take - 1) * stride + AvatarSize;
        var canvas = new System.Windows.Controls.Canvas
        {
            Width = stackWidth,
            Height = AvatarSize,
            Background = System.Windows.Media.Brushes.Transparent,
            VerticalAlignment = VerticalAlignment.Center,
        };
        for (var i = 0; i < take; i++)
        {
            var peer = peers[i];
            var avatar = new Avatar
            {
                Size = AvatarSize,
                Label = string.IsNullOrEmpty(peer.name) ? "?" : peer.name,
                PictureUrl = peer.pictureUrl,
            };
            System.Windows.Controls.Canvas.SetLeft(avatar, i * stride);
            System.Windows.Controls.Canvas.SetTop(avatar, 0);
            canvas.Children.Add(avatar);
        }
        return canvas;
    }

    private ContextMenu BuildChatContextMenu(ChatThreadSnapshot chat)
    {
        var menu = new ContextMenu();
        menu.Items.Add(MenuItem(
            chat.unreadCount > 0 ? "Mark read" : "Mark as unread",
            () => _manager.SetChatUnread(chat.chatId, chat.unreadCount == 0)
        ));
        menu.Items.Add(MenuItem(
            chat.isPinned ? "Unpin chat" : "Pin chat",
            () => _manager.SetChatPinned(chat.chatId, !chat.isPinned)
        ));
        menu.Items.Add(MenuItem(
            chat.isMuted ? "Unmute chat" : "Mute chat",
            () => _manager.SetChatMuted(chat.chatId, !chat.isMuted)
        ));
        menu.Items.Add(new Separator());
        menu.Items.Add(MenuItem("Delete", () => _manager.DeleteChat(chat.chatId)));
        return menu;
    }

    private static MenuItem MenuItem(string header, Action action)
    {
        var item = new MenuItem { Header = header };
        item.Click += (_, _) => action();
        return item;
    }

    private static string NearbySummary(DesktopNearbyPeerSnapshot[] peers)
    {
        static string Name(DesktopNearbyPeerSnapshot peer) =>
            string.IsNullOrWhiteSpace(peer.name) ? "Someone" : peer.name.Trim();

        return peers.Length switch
        {
            1 => $"{Name(peers[0])} nearby",
            2 => $"{Name(peers[0])} and {Name(peers[1])} nearby",
            _ => $"{Name(peers[0])}, {Name(peers[1])} and {peers.Length - 2} others nearby",
        };
    }

    private void OnNearby(object sender, RoutedEventArgs e)
    {
        _showingNearby = true;
        _manager.PrepareNearbyForUserTap();
        _renderedScreenKey = null;
        Refresh();
    }

    private static string WifiStatusLabel(string status) =>
        status switch
        {
            "Local network unavailable" => "Wi-Fi unavailable",
            "Local network failed" => "Wi-Fi failed",
            "No local network access" => "No Wi-Fi access",
            _ => status,
        };

    private void OnProfile(object sender, RoutedEventArgs e) =>
        PushMain(new Screen.Settings());

    private void OnNewChat(object sender, RoutedEventArgs e) =>
        PushMain(new Screen.NewChat());

    private void OnNewGroup(object sender, RoutedEventArgs e) =>
        PushMain(new Screen.NewGroup());

    private void OnInvite(object sender, RoutedEventArgs e) =>
        PushMain(new Screen.CreateInvite());

    private void OnSettings(object sender, RoutedEventArgs e) =>
        PushMain(new Screen.Settings());

    private void PushMain(Screen screen)
    {
        _showingNearby = false;
        _manager.Push(screen);
    }
}
