using System;
using System.ComponentModel;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using IrisChat.Bindings;
using IrisChat.Views;

namespace IrisChat.Chrome;

public partial class DesktopShell : UserControl
{
    private const double NearbyRowContentHeight = 64;
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
                MainHost.Content = new NearbyView(OpenNearbyPeer);
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

        if (query.Length == 0)
        {
            var showNearby = true;
            var pinned = chats.Where(chat => chat.isPinned).ToArray();
            var unpinned = chats.Where(chat => !chat.isPinned).ToArray();
            var sectionCount = (showNearby ? 1 : 0)
                + (pinned.Length > 0 ? 1 : 0)
                + ((unpinned.Length > 0 || chats.Length == 0) ? 1 : 0);

            if (showNearby)
            {
                AddSidebarSectionHeader("Nearby", sectionCount);
                ChatRows.Items.Add(BuildNearbyRow());
            }

            if (pinned.Length > 0)
            {
                AddSidebarSectionHeader("Pinned", sectionCount);
                foreach (var chat in pinned)
                    ChatRows.Items.Add(BuildChatRow(chat, activeChatId));
            }

            if (chats.Length == 0)
            {
                AddSidebarSectionHeader("Chats", sectionCount);
                ChatRows.Items.Add(BuildEmptyChatsRow());
            }
            else if (unpinned.Length > 0)
            {
                AddSidebarSectionHeader("Chats", sectionCount);
                foreach (var chat in unpinned)
                    ChatRows.Items.Add(BuildChatRow(chat, activeChatId));
            }
            return;
        }

        var visibleChats = chats.Where(chat => ChatMatchesQuery(chat, query)).ToArray();

        foreach (var chat in visibleChats)
        {
            ChatRows.Items.Add(BuildChatRow(chat, activeChatId));
        }

        if (query.Length > 0 && visibleChats.Length == 0)
        {
            ChatRows.Items.Add(BuildSearchEmptyRow());
        }
    }

    private void AddSidebarSectionHeader(string title, int sectionCount)
    {
        if (sectionCount <= 1) return;
        ChatRows.Items.Add(BuildSidebarSectionHeader(title));
    }

    private FrameworkElement BuildSidebarSectionHeader(string title) =>
        new TextBlock
        {
            Text = title,
            Foreground = (System.Windows.Media.Brush)FindResource("TextMuted"),
            FontSize = 12,
            FontWeight = FontWeights.SemiBold,
            Margin = new Thickness(12, 12, 12, 4),
        };

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

    private FrameworkElement BuildEmptyChatsRow() =>
        new Border
        {
            Padding = new Thickness(12, 20, 12, 20),
            Child = new TextBlock
            {
                Text = "No chats yet",
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
            || ContainsSearch(chat.nickname, term)
            || ContainsSearch(chat.profileName, term)
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
        var nearbyEnabled = _manager.Preferences.nearbyEnabled;
        var active = nearbyEnabled && snapshot.visible;
        var peers = nearbyEnabled ? snapshot.peers : Array.Empty<DesktopNearbyPeerSnapshot>();

        var grid = new Grid
        {
            Margin = new Thickness(10, 8, 10, 8),
            Height = NearbyRowContentHeight,
            VerticalAlignment = VerticalAlignment.Top,
        };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        if (peers.Length > 0)
        {
            var iconButton = BuildWirelessIconButton(active);
            Grid.SetColumn(iconButton, 0);
            var strip = BuildNearbyAvatarStrip(peers);
            Grid.SetColumn(strip, 1);
            grid.Children.Add(iconButton);
            grid.Children.Add(strip);
            return grid;
        }

        var leading = BuildWirelessIcon(active);
        Grid.SetColumn(leading, 0);
        grid.Children.Add(leading);

        var text = new TextBlock
        {
            Text = !nearbyEnabled ? "Off" : active ? "No users nearby" : "Tap to enable",
            Foreground = (System.Windows.Media.Brush)FindResource("TextMuted"),
            FontSize = 13,
            Margin = new Thickness(12, 0, 0, 0),
            VerticalAlignment = VerticalAlignment.Center,
            TextTrimming = TextTrimming.CharacterEllipsis,
        };
        Grid.SetColumn(text, 1);
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
        AttachLongPress(button, ToggleNearbyMaster);
        return button;
    }

    // 44px round wireless badge that always occupies the leading slot of
    // the Nearby chat-list row, so the row visually lines up with regular
    // chat avatars.
    private FrameworkElement BuildWirelessIcon(bool active)
    {
        const double Size = 44;
        return new System.Windows.Controls.Border
        {
            Width = Size,
            Height = Size,
            CornerRadius = new CornerRadius(Size / 2),
            Background = active ? NearbyActiveBrush() : (System.Windows.Media.Brush)FindResource("PanelAlt"),
            VerticalAlignment = VerticalAlignment.Top,
            Child = new TextBlock
            {
                // Segoe MDL2 Assets E701 = wireless / wifi fan
                Text = "\uE701",
                FontFamily = new System.Windows.Media.FontFamily("Segoe MDL2 Assets"),
                FontSize = 20,
                Foreground = active
                    ? System.Windows.Media.Brushes.White
                    : (System.Windows.Media.Brush)FindResource("TextMuted"),
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
            },
        };
    }

    private static System.Windows.Media.Brush NearbyActiveBrush() =>
        new System.Windows.Media.SolidColorBrush(System.Windows.Media.Color.FromRgb(0x22, 0x67, 0xF5));

    private Button BuildWirelessIconButton(bool active)
    {
        var button = new Button
        {
            Style = (Style)FindResource("GhostButton"),
            Padding = new Thickness(0),
            Width = 44,
            Height = 44,
            Content = BuildWirelessIcon(active),
            ToolTip = "Nearby",
            VerticalAlignment = VerticalAlignment.Top,
        };
        button.Click += OnNearby;
        AttachLongPress(button, ToggleNearbyMaster);
        return button;
    }

    private void ToggleNearbyMaster() =>
        _manager.SetNearbyEnabled(!_manager.Preferences.nearbyEnabled);

    private static void AttachLongPress(Button button, Action action)
    {
        DispatcherTimer? timer = null;
        var fired = false;

        button.PreviewMouseLeftButtonDown += (_, _) =>
        {
            fired = false;
            timer?.Stop();
            timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(500) };
            timer.Tick += (_, _) =>
            {
                timer?.Stop();
                fired = true;
                action();
            };
            timer.Start();
        };
        button.PreviewMouseLeftButtonUp += (_, e) =>
        {
            timer?.Stop();
            timer = null;
            if (fired) e.Handled = true;
        };
        button.MouseLeave += (_, _) =>
        {
            timer?.Stop();
            timer = null;
        };
    }

    private FrameworkElement BuildNearbyAvatarStrip(DesktopNearbyPeerSnapshot[] peers)
    {
        const double AvatarSize = 44;
        var panel = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            VerticalAlignment = VerticalAlignment.Top,
        };

        foreach (var peer in peers)
        {
            var name = NearbyPeerNames.Resolve(_manager, peer);
            var stack = new StackPanel
            {
                Orientation = Orientation.Vertical,
                Width = 64,
                VerticalAlignment = VerticalAlignment.Top,
            };
            stack.Children.Add(new Avatar
            {
                Size = AvatarSize,
                Label = name,
                PictureUrl = peer.pictureUrl,
                HorizontalAlignment = HorizontalAlignment.Center,
            });
            stack.Children.Add(new TextBlock
            {
                Text = NearbyPeerNames.Short(name),
                Foreground = (System.Windows.Media.Brush)FindResource("TextMuted"),
                FontSize = 11,
                TextTrimming = TextTrimming.CharacterEllipsis,
                TextAlignment = TextAlignment.Center,
                Margin = new Thickness(0, 4, 0, 0),
            });
            var button = new Button
            {
                Style = (Style)FindResource("GhostButton"),
                Padding = new Thickness(0),
                Margin = new Thickness(0, 0, 10, 0),
                Width = 64,
                ToolTip = name,
                IsEnabled = !string.IsNullOrWhiteSpace(peer.ownerPubkeyHex),
                Content = stack,
                VerticalAlignment = VerticalAlignment.Top,
            };
            if (!string.IsNullOrWhiteSpace(peer.ownerPubkeyHex))
            {
                var peerForClick = peer;
                button.Click += (_, _) => OpenNearbyPeer(peerForClick);
                AttachLongPress(button, () => ShowNearbyPeerProfile(peerForClick));
            }
            panel.Children.Add(button);
        }

        return new ScrollViewer
        {
            Content = panel,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
            VerticalScrollBarVisibility = ScrollBarVisibility.Disabled,
            Margin = new Thickness(12, 0, 0, 0),
            Height = NearbyRowContentHeight,
            VerticalAlignment = VerticalAlignment.Top,
        };
    }

    private void OpenNearbyPeer(DesktopNearbyPeerSnapshot peer)
    {
        var owner = peer.ownerPubkeyHex;
        if (string.IsNullOrWhiteSpace(owner)) return;

        if (IsKnownDirectChat(owner))
        {
            _showingNearby = false;
            _manager.OpenChat(owner);
            return;
        }

        ShowNearbyPeerProfile(peer);
    }

    private void ShowNearbyPeerProfile(DesktopNearbyPeerSnapshot peer) =>
        NearbyPeerProfileWindow.Show(
            Window.GetWindow(this),
            _manager,
            peer,
            owner =>
            {
                _showingNearby = false;
                _manager.OpenChat(owner);
            }
        );

    private bool IsKnownDirectChat(string ownerPubkeyHex) =>
        _manager.ChatList.Any(chat =>
            chat.kind == ChatKind.Direct &&
            string.Equals(chat.chatId, ownerPubkeyHex, StringComparison.OrdinalIgnoreCase)
        );

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
        menu.Items.Add(MenuItem("Delete", () => ConfirmDeleteChat(chat)));
        return menu;
    }

    private void ConfirmDeleteChat(ChatThreadSnapshot chat)
    {
        var result = MessageBox.Show(
            Window.GetWindow(this),
            "This removes messages from this device.",
            "Delete chat?",
            MessageBoxButton.OKCancel,
            MessageBoxImage.Warning
        );
        if (result == MessageBoxResult.OK)
            _manager.DeleteChat(chat.chatId);
    }

    private static MenuItem MenuItem(string header, Action action)
    {
        var item = new MenuItem { Header = header };
        item.Click += (_, _) => action();
        return item;
    }

    private void OnNearby(object sender, RoutedEventArgs e)
    {
        _showingNearby = true;
        _renderedScreenKey = null;
        Refresh();
    }

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
