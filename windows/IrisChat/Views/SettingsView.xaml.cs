using System;
using System.ComponentModel;
using System.IO;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using IrisChat.Bindings;

namespace IrisChat.Views;

public partial class SettingsView : UserControl
{
    private const string IrisSourceUrl =
        "https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/iris-chat-rs";

    private bool _suppressToggleDispatch;

    public SettingsView()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            Refresh();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => Refresh();

    private void Refresh()
    {
        var account = App.CurrentManager.Account;
        if (account != null)
        {
            ProfileAvatar.Label = string.IsNullOrEmpty(account.displayName) ? "Iris user" : account.displayName;
            ProfileAvatar.PictureUrl = account.pictureUrl;
            if (!ProfileNameInput.IsKeyboardFocused)
                ProfileNameInput.Text = account.displayName;
            NpubText.Text = "Signed in";
            ExportOwnerKeyButton.Visibility = account.hasOwnerSigningAuthority
                ? Visibility.Visible
                : Visibility.Collapsed;
        }

        var prefs = App.CurrentManager.Preferences;
        _suppressToggleDispatch = true;
        TypingToggle.IsChecked = prefs.sendTypingIndicators;
        ReceiptsToggle.IsChecked = prefs.sendReadReceipts;
        NotificationsToggle.IsChecked = prefs.desktopNotificationsEnabled;
        StartupToggle.IsChecked = prefs.startupAtLoginEnabled;
        NearbyLanToggle.IsChecked = prefs.nearbyLanEnabled;
        AutoCheckUpdatesToggle.IsChecked = App.CurrentManager.AutoCheckUpdates;
        AutoInstallUpdatesToggle.IsChecked = App.CurrentManager.AutoInstallUpdates;
        StartupToggle.Visibility = PlatformStartupAtLogin.IsSupported ? Visibility.Visible : Visibility.Collapsed;
        _suppressToggleDispatch = false;

        RebuildRelays(prefs);

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
        UpdateStatusText.Text = App.CurrentManager.UpdateStatus;
        CheckUpdatesButton.IsEnabled = !App.CurrentManager.UpdateChecking && !App.CurrentManager.UpdateInstalling;
        InstallUpdateButton.IsEnabled = App.CurrentManager.UpdateInstallEnabled;
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
                Foreground = (Brush)Application.Current.Resources["TextPrimary"],
                FontSize = 13,
                VerticalAlignment = VerticalAlignment.Center,
                FontFamily = new FontFamily("Consolas, 'Cascadia Mono', monospace"),
            };
            Grid.SetColumn(text, 0);

            var remove = new Button
            {
                Style = (Style)FindResource("GhostButton"),
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

    private void OnReceiptsChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetReadReceiptsEnabled(ReceiptsToggle.IsChecked == true);
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

    private async void OnCheckUpdates(object sender, RoutedEventArgs e)
    {
        await RunUpdateAction(() => App.CurrentManager.CheckForUpdatesAsync());
    }

    private async void OnInstallUpdate(object sender, RoutedEventArgs e)
    {
        await RunUpdateAction(() => App.CurrentManager.InstallUpdateAsync());
    }

    private async Task RunUpdateAction(Func<Task> action)
    {
        try
        {
            await action();
        }
        finally
        {
            Refresh();
        }
    }

    private void OnSaveProfile(object sender, RoutedEventArgs e)
    {
        var name = ProfileNameInput.Text?.Trim();
        if (string.IsNullOrEmpty(name)) return;
        App.CurrentManager.UpdateProfileMetadata(name, App.CurrentManager.Account?.pictureUrl);
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

    private void OnManageDevices(object sender, RoutedEventArgs e) =>
        App.CurrentManager.Push(new Screen.DeviceRoster());

    private void OnCopyUserId(object sender, RoutedEventArgs e)
    {
        var npub = App.CurrentManager.Account?.npub;
        if (!string.IsNullOrEmpty(npub)) App.CurrentManager.CopyToClipboard(npub);
    }

    private void OnExportOwner(object sender, RoutedEventArgs e)
    {
        var nsec = App.CurrentManager.ExportOwnerNsec();
        if (string.IsNullOrEmpty(nsec)) { App.CurrentManager.ShowToast("Key unavailable"); return; }
        App.CurrentManager.CopyToClipboard(nsec!);
    }

    private void OnExportDevice(object sender, RoutedEventArgs e)
    {
        var nsec = App.CurrentManager.ExportDeviceNsec();
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
            "This removes your secret keys, messages, and cached files from this device.",
            "Delete app data?",
            MessageBoxButton.OKCancel,
            MessageBoxImage.Warning
        );
        return result == MessageBoxResult.OK;
    }
}
