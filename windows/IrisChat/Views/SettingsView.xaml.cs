using System;
using System.ComponentModel;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using IrisChat.Bindings;

namespace IrisChat.Views;

public partial class SettingsView : UserControl
{
    private const string IrisSourceUrl =
        "https://git.iris.to/#/npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap/iris-chat-rs";

    private static string IrisChatProfileUrl(string npub) => $"https://chat.iris.to/#/{npub}";

    private (string Id, string Name, string UserId)[]? _renderedBlockedPeople;
    private bool _suppressToggleDispatch;
    private string _selectedPage = "Profile";
    private bool _profileQrVisible;
    private string? _profileName;
    private string? _profileAbout;

    public SettingsView()
    {
        InitializeComponent();
        SettingsNavigation.SelectedIndex = 0;
        PreviewKeyDown += (_, e) =>
        {
            if (e.Key == Key.BrowserBack || (e.Key == Key.System && e.SystemKey == Key.Left
                && Keyboard.Modifiers == ModifierKeys.Alt))
            {
                App.CurrentManager.NavigateBack();
                e.Handled = true;
            }
        };
        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            Refresh();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    // Progress arrives often; repaint only the update section for it.
    private void OnChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName == nameof(AppManager.UpdateView))
        {
            RenderUpdate();
        }
        else
        {
            Refresh();
        }
    }

    private void Refresh()
    {
        var account = App.CurrentManager.Account;
        if (account != null)
        {
            ProfileAvatar.Label = string.IsNullOrEmpty(account.displayName) ? "Iris user" : account.displayName;
            ProfileAvatar.PictureUrl = account.pictureUrl;
            // Refresh untouched fields, preserving edits even when another
            // category has focus. The next snapshot acknowledges saved values.
            if (_profileName == null || ProfileNameInput.Text == _profileName)
                ProfileNameInput.Text = account.displayName;
            if (_profileAbout == null || ProfileAboutInput.Text == _profileAbout)
                ProfileAboutInput.Text = account.about ?? string.Empty;
            _profileName = account.displayName;
            _profileAbout = account.about ?? string.Empty;
            NpubText.Text = "Signed in";
            ProfileQrName.Text = string.IsNullOrEmpty(account.displayName) ? "User ID" : account.displayName;
            ProfileQr.Text = _profileQrVisible ? IrisChatProfileUrl(account.npub) : null;
            ProfileQrPanel.Visibility = _profileQrVisible ? Visibility.Visible : Visibility.Collapsed;
            ExportOwnerKeyButton.Visibility = account.hasOwnerSigningAuthority
                ? Visibility.Visible
                : Visibility.Collapsed;
        }
        else
        {
            ProfileQrPanel.Visibility = Visibility.Collapsed;
            ProfileQr.Text = null;
        }

        var prefs = App.CurrentManager.Preferences;
        _suppressToggleDispatch = true;
        AcceptChatRequestsToggle.IsChecked = prefs.acceptUnknownDirectMessages;
        NetworkRequestsToggle.IsChecked = prefs.acceptSocialGraphDirectMessages;
        OvermutedRequestsToggle.IsChecked = prefs.acceptOvermutedDirectMessages;
        TypingToggle.IsChecked = prefs.sendTypingIndicators;
        ReceiptsToggle.IsChecked = prefs.sendReadReceipts;
        AllowMessageDeletionToggle.IsChecked = prefs.allowMessageDeletionByOthers;
        HideBlockedGroupMessagesToggle.IsChecked = prefs.hideBlockedGroupMessages;
        NotificationsToggle.IsChecked = prefs.desktopNotificationsEnabled;
        StartupToggle.IsChecked = prefs.startupAtLoginEnabled;
        NearbyEnabledToggle.IsChecked = prefs.nearbyEnabled;
        NearbyLanToggle.IsChecked = prefs.nearbyLanEnabled;
        NearbyShowInChatListToggle.IsChecked = prefs.nearbyShowInChatList;
        NearbyLanToggle.Visibility = Visibility.Visible;
        NearbyLanToggle.IsEnabled = prefs.nearbyEnabled;
        ImageProxyToggle.IsChecked = prefs.imageProxyEnabled;
        ImageProxyFallbackToggle.IsChecked = prefs.imageProxyFallbackEnabled;
        ImageProxyFallbackToggle.IsEnabled = prefs.imageProxyEnabled;
        AutoCheckUpdatesToggle.IsChecked = App.CurrentManager.AutoCheckUpdates;
        AutoInstallUpdatesToggle.IsChecked = App.CurrentManager.AutoInstallUpdates;
        StartupToggle.Visibility = PlatformStartupAtLogin.IsSupported ? Visibility.Visible : Visibility.Collapsed;
        _suppressToggleDispatch = false;

        if (!ImageProxyUrlInput.IsKeyboardFocused)
            ImageProxyUrlInput.Text = prefs.imageProxyUrl;
        if (!ImageProxyKeyInput.IsKeyboardFocused)
            ImageProxyKeyInput.Text = prefs.imageProxyKeyHex;
        if (!ImageProxySaltInput.IsKeyboardFocused)
            ImageProxySaltInput.Text = prefs.imageProxySaltHex;

        RebuildRelays(prefs);
        RebuildBlockedPeople();

        VersionText.Text = $"Version {App.CurrentManager.BuildSummary()}";
        RelaySetText.Text = string.Empty;

        var net = App.CurrentManager.NetworkStatus;
        if (net != null)
        {
            NetworkText.Text =
                $"Network {(net.syncing ? "syncing" : "idle")} · {net.relayUrls.Length} servers · {net.recentEventCount} updates";
        }
        else
        {
            NetworkText.Text = string.Empty;
        }

        UpdateVersionText.Text = $"Current version {App.CurrentManager.AppVersion()}";
        RenderUpdate();
    }

    private void RenderUpdate()
    {
        var view = App.CurrentManager.UpdateView;
        UpdateStatusText.Text = view.status;
        UpdateStatusText.Visibility = string.IsNullOrEmpty(view.status) ? Visibility.Collapsed : Visibility.Visible;
        UpdateDetailText.Text = view.detail ?? string.Empty;
        UpdateDetailText.Visibility = view.detail is null ? Visibility.Collapsed : Visibility.Visible;
        UpdateProgress.Visibility = view.busy ? Visibility.Visible : Visibility.Collapsed;
        UpdateProgress.IsIndeterminate = view.progress is null;
        UpdateProgress.Value = view.progress ?? 0;
        CheckUpdatesButton.IsEnabled = view.canCheck && !view.busy;
        var action = view.busy ? null : view.actionLabel;
        InstallUpdateButton.Visibility = action is null ? Visibility.Collapsed : Visibility.Visible;
        InstallUpdateButtonText.Text = action ?? string.Empty;
        AutomationProperties.SetName(InstallUpdateButton, action ?? string.Empty);
        var checkedAt = view.lastCheckedSecs is { } secs && !view.busy
            ? DateTimeOffset.FromUnixTimeSeconds((long)secs).ToLocalTime()
            : (DateTimeOffset?)null;
        UpdateLastCheckedText.Text = checkedAt is { } at ? $"Last checked {at:g}" : string.Empty;
        UpdateLastCheckedText.Visibility = checkedAt is null ? Visibility.Collapsed : Visibility.Visible;
    }

    private void RebuildBlockedPeople()
    {
        var people = App.CurrentManager.State.blockedPeople
            .Select(person => (Id: person.ownerPubkeyHex, Name: person.displayLabel, UserId: person.userId))
            .ToArray();
        if (_renderedBlockedPeople != null && _renderedBlockedPeople.SequenceEqual(people)) return;
        _renderedBlockedPeople = people;
        BlockedPeopleList.Children.Clear();
        if (people.Length == 0)
            BlockedPeopleList.Children.Add(new TextBlock { Text = "No blocked people", Style = (Style)FindResource("MutedText") });
        foreach (var person in people)
        {
            var row = new Grid { Margin = new Thickness(0, 4, 0, 4) };
            row.ColumnDefinitions.Add(new ColumnDefinition());
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var identity = new StackPanel { VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 12, 0) };
            identity.Children.Add(new TextBlock { Text = person.Name, TextTrimming = TextTrimming.CharacterEllipsis });
            identity.Children.Add(new TextBlock { Text = person.UserId, TextTrimming = TextTrimming.CharacterEllipsis, Style = (Style)FindResource("MutedText") });
            row.Children.Add(identity);
            var unblock = new Button { Content = "Unblock", Style = (Style)FindResource("SettingsButton"), VerticalAlignment = VerticalAlignment.Center };
            System.Windows.Automation.AutomationProperties.SetName(unblock, $"Unblock {person.Name}");
            unblock.Click += (_, _) => App.CurrentManager.SetUserBlocked(person.Id, false);
            Grid.SetColumn(unblock, 1);
            row.Children.Add(unblock);
            BlockedPeopleList.Children.Add(row);
        }
    }

    private void OnBack(object sender, RoutedEventArgs e) => App.CurrentManager.NavigateBack();

    private void OnSettingsSizeChanged(object sender, SizeChangedEventArgs e)
    {
        var compact = ActualWidth < 760;
        NavigationColumn.Width = new GridLength(compact ? 72 : 240);
        SettingsNavigation.Tag = compact ? "Compact" : null;
        PageContent.Margin = compact ? new Thickness(16, 20, 16, 24) : new Thickness(32, 20, 32, 32);
    }

    private void OnSettingsSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (SettingsNavigation.SelectedItem is not ListBoxItem { Tag: string page } item) return;
        _selectedPage = page;
        PageTitle.Text = System.Windows.Automation.AutomationProperties.GetName(item);
        UpdateSettingsPageVisibility();
        PageScroll.ScrollToTop();
    }

    private void OnWindowsColors(object sender, RoutedEventArgs e)
    {
        if (!PlatformDocumentOpener.OpenUrl("ms-settings:colors"))
            App.CurrentManager.ShowToast("Could not open Windows color settings");
    }

    private void UpdateSettingsPageVisibility()
    {
        foreach (var page in new FrameworkElement[]
        {
            ProfilePage,
            DevicesPage,
            GeneralPage,
            MessagingPage,
            NotificationsPage,
            MediaPage,
            NearbyPage,
            MessageServersPage,
            SecurityPage,
            UpdatesPage,
            AboutPage,
            SupportPage,
            AccountDataPage,
        })
        {
            page.Visibility = Visibility.Collapsed;
            page.Margin = new Thickness(0);
        }

        var selected = _selectedPage switch
        {
            "Devices" => DevicesPage,
            "General" => GeneralPage,
            "Messaging" => MessagingPage,
            "Notifications" => NotificationsPage,
            "Media" => MediaPage,
            "Nearby" => NearbyPage,
            "MessageServers" => MessageServersPage,
            "Security" => SecurityPage,
            "Updates" => UpdatesPage,
            "About" => AboutPage,
            "Support" => SupportPage,
            "AccountData" => AccountDataPage,
            _ => ProfilePage,
        };
        selected.Visibility = Visibility.Visible;
    }

    private void RebuildRelays(PreferencesSnapshot prefs)
    {
        RelaysList.Items.Clear();
        foreach (var url in prefs.nostrRelayUrls ?? Array.Empty<string>())
        {
            var captured = url;

            var grid = new Grid();
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

            var text = new TextBlock
            {
                Text = url,
                TextTrimming = TextTrimming.CharacterEllipsis,
                ToolTip = url,
                Foreground = (Brush)Application.Current.Resources["TextPrimary"],
                FontSize = 13,
                VerticalAlignment = VerticalAlignment.Center,
                FontFamily = new FontFamily("Consolas, 'Cascadia Mono', monospace"),
            };
            Grid.SetColumn(text, 0);

            var remove = new Button
            {
                Style = (Style)FindResource("SettingsButton"),
                Content = new TextBlock
                {
                    Text = "Remove",
                    Foreground = (Brush)Application.Current.Resources["Danger"],
                },
                Padding = new Thickness(8, 4, 8, 4),
            };
            remove.Click += (_, _) => App.CurrentManager.RemoveNostrRelay(captured);
            Grid.SetColumn(remove, 1);

            grid.Children.Add(text);
            grid.Children.Add(remove);

            var border = new Border
            {
                Background = Brushes.Transparent,
                Padding = new Thickness(8, 6, 8, 6),
                Child = grid,
            };
            RelaysList.Items.Add(border);
        }
    }

    private void OnTypingChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetTypingIndicatorsEnabled(TypingToggle.IsChecked == true);
    }

    private void OnAcceptChatRequestsChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetAcceptUnknownDirectMessages(AcceptChatRequestsToggle.IsChecked == true);
    }

    private void OnNetworkRequestsChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetAcceptSocialGraphDirectMessages(NetworkRequestsToggle.IsChecked == true);
    }

    private void OnOvermutedRequestsChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetAcceptOvermutedDirectMessages(OvermutedRequestsToggle.IsChecked == true);
    }

    private void OnReceiptsChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetReadReceiptsEnabled(ReceiptsToggle.IsChecked == true);
    }

    private void OnHideBlockedGroupMessagesChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetHideBlockedGroupMessages(HideBlockedGroupMessagesToggle.IsChecked == true);
    }

    private void OnAllowMessageDeletionChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetAllowMessageDeletionByOthers(AllowMessageDeletionToggle.IsChecked == true);
    }

    private void OnNotificationsChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetDesktopNotificationsEnabled(NotificationsToggle.IsChecked == true);
    }

    private void OnStartupChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetStartupAtLoginEnabled(StartupToggle.IsChecked == true);
    }

    private void OnNearbyLanChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetNearbyLanEnabled(NearbyLanToggle.IsChecked == true);
    }

    private void OnNearbyEnabledChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetNearbyEnabled(NearbyEnabledToggle.IsChecked == true);
    }

    private void OnNearbyShowInChatListChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetNearbyShowInChatList(NearbyShowInChatListToggle.IsChecked == true);
    }

    private void OnImageProxyChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetImageProxyEnabled(ImageProxyToggle.IsChecked == true);
    }

    private void OnImageProxyFallbackChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetImageProxyFallbackEnabled(ImageProxyFallbackToggle.IsChecked == true);
    }

    private void OnImageProxyUrlCommitted(object sender, RoutedEventArgs e) =>
        App.CurrentManager.SetImageProxyUrl(ImageProxyUrlInput.Text ?? string.Empty);

    private void OnImageProxyKeyCommitted(object sender, RoutedEventArgs e) =>
        App.CurrentManager.SetImageProxyKeyHex(ImageProxyKeyInput.Text ?? string.Empty);

    private void OnImageProxySaltCommitted(object sender, RoutedEventArgs e) =>
        App.CurrentManager.SetImageProxySaltHex(ImageProxySaltInput.Text ?? string.Empty);

    private void OnResetImageProxy(object sender, RoutedEventArgs e)
    {
        App.CurrentManager.ResetImageProxySettings();
        Refresh();
    }

    private void OnAutoCheckUpdatesChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.AutoCheckUpdates = AutoCheckUpdatesToggle.IsChecked == true;
    }

    private void OnAutoInstallUpdatesChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.AutoInstallUpdates = AutoInstallUpdatesToggle.IsChecked == true;
    }

    private void OnCheckUpdates(object sender, RoutedEventArgs e) =>
        App.CurrentManager.CheckForUpdates();

    private void OnInstallUpdate(object sender, RoutedEventArgs e) =>
        App.CurrentManager.PerformUpdateAction();

    private void OnNoteToSelf(object sender, RoutedEventArgs e)
    {
        if (App.CurrentManager.Account is { } account)
            App.CurrentManager.OpenChat(account.publicKeyHex);
    }

    private void OnSaveProfile(object sender, RoutedEventArgs e)
    {
        var name = ProfileNameInput.Text?.Trim();
        if (string.IsNullOrEmpty(name)) return;
        App.CurrentManager.UpdateProfileMetadata(name, App.CurrentManager.Account?.pictureUrl, ProfileAboutInput.Text);
    }

    private void OnPickPicture(object sender, RoutedEventArgs e)
    {
        var file = PlatformFilePicker.PickImage("Choose profile picture");
        if (string.IsNullOrEmpty(file)) return;
        App.CurrentManager.UploadProfilePicture(file!);
    }

    private void OnAddRelay(object sender, RoutedEventArgs e)
    {
        var url = NewRelayInput.Text?.Trim();
        if (string.IsNullOrEmpty(url)) return;
        App.CurrentManager.AddNostrRelay(url);
        NewRelayInput.Clear();
    }

    private void OnResetRelays(object sender, RoutedEventArgs e) =>
        App.CurrentManager.ResetNostrRelays();

    private void OnCopyUserId(object sender, RoutedEventArgs e)
    {
        var npub = App.CurrentManager.Account?.npub;
        if (!string.IsNullOrEmpty(npub)) App.CurrentManager.CopyToClipboard(npub);
    }

    private void OnCopyProfileLink(object sender, RoutedEventArgs e)
    {
        var npub = App.CurrentManager.Account?.npub;
        if (!string.IsNullOrEmpty(npub)) App.CurrentManager.CopyToClipboard(IrisChatProfileUrl(npub));
    }

    private void OnToggleProfileQr(object sender, RoutedEventArgs e)
    {
        _profileQrVisible = !_profileQrVisible;
        Refresh();
    }

    private void OnExportOwner(object sender, RoutedEventArgs e)
    {
        var nsec = App.CurrentManager.ExportOwnerNsec();
        if (string.IsNullOrEmpty(nsec)) { App.CurrentManager.ShowToast("Key unavailable"); return; }
        App.CurrentManager.CopyToClipboard(nsec!);
    }

    private void OnExportSupport(object sender, RoutedEventArgs e)
    {
        var path = PlatformFilePicker.SaveFile(
            "Save support bundle",
            $"iris-chat-support-{DateTime.Now:yyyyMMdd-HHmmss}.json",
            "JSON files (*.json)|*.json|All files (*.*)|*.*"
        );
        if (string.IsNullOrEmpty(path)) return;
        try
        {
            File.WriteAllText(path!, App.CurrentManager.SupportBundleJson());
            App.CurrentManager.ShowToast("Support bundle saved");
        }
        catch
        {
            App.CurrentManager.ShowToast("Could not save support bundle");
        }
    }

    private void OnSourceCode(object sender, RoutedEventArgs e)
    {
        if (!PlatformDocumentOpener.OpenUrl(IrisSourceUrl))
            App.CurrentManager.ShowToast("Could not open source code");
    }

    private void OnLogout(object sender, RoutedEventArgs e)
    {
        if (ConfirmDeleteAppData())
            App.CurrentManager.Logout();
    }

    private bool ConfirmDeleteAppData()
    {
        var result = MessageBox.Show(
            Window.GetWindow(this),
            "This removes your secret keys, messages, and cached files from this device. Your other devices keep their data.",
            "Log out and delete local data?",
            MessageBoxButton.OKCancel,
            MessageBoxImage.Warning
        );
        return result == MessageBoxResult.OK;
    }
}
