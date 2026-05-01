using System;
using System.ComponentModel;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
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

        // Sidebar chat list
        var chats = _manager.ChatList ?? Array.Empty<ChatThreadSnapshot>();
        var activeChatId = _activeScreen is Screen.Chat c ? c.chatId : null;
        ChatRows.Items.Clear();
        ChatRows.Items.Add(BuildNearbyRow());
        foreach (var chat in chats.OrderByDescending(c => c.lastMessageAtSecs ?? 0))
        {
            var row = new ChatRow { Chat = chat, IsActive = chat.chatId == activeChatId };
            row.Activated += chosen =>
            {
                _showingNearby = false;
                _manager.OpenChat(chosen.chatId);
            };
            ChatRows.Items.Add(row);
        }

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

        var grid = new Grid { Margin = new Thickness(8, 7, 8, 7) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        var icon = new TextBlock
        {
            Text = "N",
            FontSize = 18,
            Foreground = (System.Windows.Media.Brush)FindResource("TextMuted"),
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(3, 0, 12, 0),
        };
        Grid.SetColumn(icon, 0);

        var text = new StackPanel { Orientation = Orientation.Vertical };
        text.Children.Add(new TextBlock
        {
            Text = "Nearby",
            Foreground = (System.Windows.Media.Brush)FindResource("TextPrimary"),
            FontWeight = FontWeights.SemiBold,
        });
        text.Children.Add(new TextBlock
        {
            Text = subtitle,
            Foreground = (System.Windows.Media.Brush)FindResource("TextMuted"),
            FontSize = 12,
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        Grid.SetColumn(text, 1);

        grid.Children.Add(icon);
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
