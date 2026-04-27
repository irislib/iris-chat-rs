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
                ? account.npub
                : account.displayName;
            ProfileAvatar.PictureUrl = account.pictureUrl;
        }

        // Sidebar chat list
        var chats = _manager.ChatList ?? Array.Empty<ChatThreadSnapshot>();
        var activeChatId = _activeScreen is Screen.Chat c ? c.chatId : null;
        ChatRows.Items.Clear();
        foreach (var chat in chats.OrderByDescending(c => c.lastMessageAtSecs ?? 0))
        {
            var row = new ChatRow { Chat = chat, IsActive = chat.chatId == activeChatId };
            row.Activated += chosen => _manager.OpenChat(chosen.chatId);
            ChatRows.Items.Add(row);
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

    private void OnProfile(object sender, RoutedEventArgs e) =>
        _manager.Push(new Screen.Settings());

    private void OnNewChat(object sender, RoutedEventArgs e) =>
        _manager.Push(new Screen.NewChat());

    private void OnNewGroup(object sender, RoutedEventArgs e) =>
        _manager.Push(new Screen.NewGroup());

    private void OnInvite(object sender, RoutedEventArgs e) =>
        _manager.Push(new Screen.CreateInvite());

    private void OnSettings(object sender, RoutedEventArgs e) =>
        _manager.Push(new Screen.Settings());
}
