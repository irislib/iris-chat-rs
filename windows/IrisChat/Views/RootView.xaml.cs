using System;
using System.ComponentModel;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using IrisChat.Bindings;
using IrisChat.Chrome;

namespace IrisChat.Views;

public partial class RootView : UserControl
{
    private AppManager? _manager;
    private string? _currentScreenKey;

    public RootView()
    {
        InitializeComponent();
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        _manager = App.CurrentManager;
        _manager.PropertyChanged += OnManagerChanged;
        Refresh();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        if (_manager != null) _manager.PropertyChanged -= OnManagerChanged;
        _manager = null;
    }

    private void OnManagerChanged(object? sender, PropertyChangedEventArgs e) => Refresh();

    private void Refresh()
    {
        if (_manager == null) return;

        LoadingOverlay.Visibility = _manager.BootstrapInFlight ? Visibility.Visible : Visibility.Collapsed;
        if (_manager.BootstrapInFlight)
        {
            _currentScreenKey = "__loading";
            ScreenHost.Content = null;
            return;
        }

        var toast = _manager.ToastMessage;
        if (!string.IsNullOrEmpty(toast))
        {
            ToastText.Text = toast;
            ToastHost.Visibility = Visibility.Visible;
        }
        else
        {
            ToastHost.Visibility = Visibility.Collapsed;
        }

        var screen = _manager.ActiveScreen;
        var key = ScreenKey(screen);
        var usesDesktopShell = UsesDesktopShell(screen, _manager.Account != null);

        // Avoid rebuilding the screen tree on every state notification when the
        // active screen hasn't changed — the inner controls listen to the
        // manager themselves.
        if (key == _currentScreenKey)
        {
            // Update the chrome's surface (title, badge, leading/trailing) in
            // place if we're using a NavigationShell.
            if (ScreenHost.Content is NavigationShell shell)
            {
                shell.Title = ScreenTitle(screen);
                shell.CanGoBack = _manager.CanNavigateBack;
                shell.BackBadgeCount = ComputeBackBadge();
            }
            return;
        }

        _currentScreenKey = key;
        ScreenHost.Content = BuildScreenChrome(screen, usesDesktopShell);
    }

    private FrameworkElement BuildScreenChrome(Screen screen, bool desktop)
    {
        if (desktop)
        {
            return new DesktopShell(_manager!, screen);
        }

        var body = BuildScreenBody(screen);
        var shell = new NavigationShell
        {
            Title = ScreenTitle(screen),
            CanGoBack = _manager!.CanNavigateBack,
            BackBadgeCount = ComputeBackBadge(),
            Body = body,
        };
        shell.BackRequested += () => _manager.NavigateBack();
        return shell;
    }

    private FrameworkElement BuildScreenBody(Screen screen) =>
        screen switch
        {
            Screen.Welcome => new WelcomeView(),
            Screen.CreateAccount => new CreateAccountView(),
            Screen.RestoreAccount => new RestoreAccountView(),
            Screen.AddDevice => new AddDeviceView(awaitingApproval: false),
            Screen.AwaitingDeviceApproval => new AddDeviceView(awaitingApproval: true),
            Screen.DeviceRevoked => new DeviceRevokedView(),
            Screen.ChatList => new ChatListView(),
            Screen.NewChat => new NewChatView(),
            Screen.NewGroup => new NewGroupView(),
            Screen.CreateInvite => new CreateInviteView(),
            Screen.JoinInvite => new JoinInviteView(),
            Screen.Settings => new SettingsView(),
            Screen.Chat c => new ChatView { ChatId = c.chatId },
            Screen.GroupDetails g => new GroupDetailsView { GroupId = g.groupId },
            Screen.DeviceRoster => new DeviceRosterView(),
            _ => new TextBlock { Text = "Unknown screen", Margin = new Thickness(40) },
        };

    private static bool UsesDesktopShell(Screen screen, bool signedIn) =>
        signedIn && screen switch
        {
            Screen.Welcome or Screen.CreateAccount or Screen.RestoreAccount
                or Screen.AddDevice or Screen.AwaitingDeviceApproval
                or Screen.DeviceRevoked => false,
            _ => true,
        };

    private ulong ComputeBackBadge()
    {
        if (_manager == null) return 0;
        if (_manager.ActiveScreen is not Screen.Chat chat) return 0;
        return _manager.ChatList
            .Where(c => c.chatId != chat.chatId)
            .Aggregate(0UL, (a, c) => a + c.unreadCount);
    }

    private static string ScreenTitle(Screen screen) => screen switch
    {
        Screen.Welcome => "Welcome",
        Screen.CreateAccount => "Create Profile",
        Screen.RestoreAccount => "Restore Profile",
        Screen.AddDevice => "Link Device",
        Screen.ChatList => "Chats",
        Screen.NewChat => "New Chat",
        Screen.NewGroup => "New Group",
        Screen.CreateInvite => "Invite",
        Screen.JoinInvite => "Join Chat",
        Screen.Settings => "Settings",
        Screen.Chat => "Chat",
        Screen.GroupDetails => "Group",
        Screen.DeviceRoster => "Manage Devices",
        Screen.AwaitingDeviceApproval => "Finish Linking",
        Screen.DeviceRevoked => "Device Removed",
        _ => "Iris Chat",
    };

    public static string ScreenKey(Screen screen) => screen switch
    {
        Screen.Chat c => $"chat:{c.chatId}",
        Screen.GroupDetails g => $"group:{g.groupId}",
        _ => screen.GetType().Name,
    };
}
