using System;
using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using IrisChat.Bindings;

namespace IrisChat.Views;

public partial class DeviceRosterView : UserControl
{
    private bool _isSubmittingDeviceInput;

    public DeviceRosterView()
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
        var roster = App.CurrentManager.DeviceRoster;
        DevicesList.Items.Clear();

        if (roster == null)
        {
            HeaderHint.Text = "Sign in first to manage your devices.";
            AddBlock.Visibility = Visibility.Collapsed;
            return;
        }

        AddBlock.Visibility = roster.canManageDevices ? Visibility.Visible : Visibility.Collapsed;

        HeaderHint.Text = roster.canManageDevices
            ? "These devices can use your profile."
            : "This device can view the list but cannot change it.";

        foreach (var d in roster.devices)
        {
            DevicesList.Items.Add(BuildRow(roster, d));
        }
    }

    private FrameworkElement BuildRow(DeviceRosterSnapshot roster, DeviceEntrySnapshot d)
    {
        var grid = new Grid { Margin = new Thickness(0, 0, 0, 8) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var iconText = new TextBlock
        {
            Text = "💻",
            FontSize = 18,
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(0, 0, 12, 0),
        };
        Grid.SetColumn(iconText, 0);

        var info = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        var primary = new TextBlock
        {
            Text = TitleText(d),
            Foreground = (Brush)Application.Current.Resources["TextPrimary"],
            FontWeight = FontWeights.SemiBold,
        };
        info.Children.Add(primary);

        var meta = new TextBlock
        {
            Foreground = (Brush)Application.Current.Resources["TextMuted"],
            FontSize = 12,
            Text = StatusText(d),
            Margin = new Thickness(0, 2, 0, 0),
        };
        info.Children.Add(meta);
        Grid.SetColumn(info, 1);

        var actions = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            VerticalAlignment = VerticalAlignment.Center,
        };

        if (roster.canManageDevices && !d.isCurrentDevice && d.isAuthorized)
        {
            var revoke = new Button
            {
                Style = (Style)FindResource("DangerButton"),
                Content = new TextBlock { Text = "Remove" },
                Padding = new Thickness(10, 6, 10, 6),
            };
            revoke.Click += (_, _) => App.CurrentManager.RemoveAuthorizedDevice(d.devicePubkeyHex);
            actions.Children.Add(revoke);
        }

        Grid.SetColumn(actions, 2);

        grid.Children.Add(iconText);
        grid.Children.Add(info);
        grid.Children.Add(actions);

        return new Border
        {
            Background = (Brush)Application.Current.Resources["Panel"],
            CornerRadius = new CornerRadius(14),
            Padding = new Thickness(14, 10, 14, 10),
            Margin = new Thickness(0, 0, 0, 8),
            Child = grid,
        };
    }

    private static string StatusText(DeviceEntrySnapshot d)
    {
        var status = d.isAuthorized ? (d.isStale ? "needs attention" : "linked") : "removed";
        if (!string.IsNullOrWhiteSpace(d.clientLabel))
        {
            status = $"{d.clientLabel!.Trim()} · {status}";
        }
        if (d.addedAtSecs is { } secs && secs > 0)
        {
            var t = DateTimeOffset.FromUnixTimeSeconds((long)secs).LocalDateTime;
            var ago = DateTime.Now - t;
            string when = ago.TotalMinutes < 1 ? "just now"
                       : ago.TotalHours < 1 ? $"{(int)ago.TotalMinutes}m ago"
                       : ago.TotalDays < 1 ? $"{(int)ago.TotalHours}h ago"
                       : $"{(int)ago.TotalDays}d ago";
            return $"{status} · added {when}";
        }
        return status;
    }

    private static string TitleText(DeviceEntrySnapshot d)
    {
        var label = d.deviceLabel?.Trim();
        if (!string.IsNullOrEmpty(label) && !d.isCurrentDevice) return label!;
        if (!string.IsNullOrEmpty(label) && d.isCurrentDevice) return $"This device · {label}";
        return d.isCurrentDevice ? "This device" : "Linked device";
    }

    private void OnDeviceInputChanged(object sender, TextChangedEventArgs e)
    {
        if (_isSubmittingDeviceInput) return;
        var roster = App.CurrentManager.DeviceRoster;
        if (roster?.canManageDevices != true || App.CurrentManager.Busy.updatingRoster) return;
        var input = ResolveDeviceAuthorizationInput(DeviceInput.Text);
        if (input == null) return;

        _isSubmittingDeviceInput = true;
        try
        {
            if (!input.RequiresConfirmation || ConfirmLinkDevice(input))
            {
                App.CurrentManager.AddAuthorizedDevice(input.DeviceInput);
            }
            DeviceInput.Clear();
        }
        finally
        {
            _isSubmittingDeviceInput = false;
        }
    }

    private bool ConfirmLinkDevice(ResolvedDeviceAuthorizationInput input)
    {
        var result = MessageBox.Show(
            Window.GetWindow(this),
            LinkDeviceConfirmationMessage(input),
            LinkDeviceConfirmationTitle(input),
            MessageBoxButton.OKCancel,
            MessageBoxImage.Question
        );
        return result == MessageBoxResult.OK;
    }

    private static ResolvedDeviceAuthorizationInput? ResolveDeviceAuthorizationInput(string? rawInput)
    {
        var trimmed = rawInput?.Trim();
        if (string.IsNullOrEmpty(trimmed)) return null;

        if (Native.IsDeviceApprovalBootstrap(trimmed))
        {
            return new ResolvedDeviceAuthorizationInput(trimmed, true);
        }
        return null;
    }

    private static string LinkDeviceConfirmationTitle(ResolvedDeviceAuthorizationInput input)
    {
        return "Link this device?";
    }

    private static string LinkDeviceConfirmationMessage(ResolvedDeviceAuthorizationInput input)
    {
        return "This device will be able to use your profile.";
    }

    private sealed record ResolvedDeviceAuthorizationInput(
        string DeviceInput,
        bool RequiresConfirmation
    );
}
