using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using IrisChat.Bindings;

namespace IrisChat.Views;

public partial class NewChatView : UserControl
{
    private string? _lastSubmitted;
    private bool _qrVisible;

    public NewChatView()
    {
        InitializeComponent();
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        App.CurrentManager.PropertyChanged += OnChanged;

        // Auto-create the public invite for share if we don't have one yet
        // (matches macOS task { ... } behavior).
        if (App.CurrentManager.PublicInvite == null && !App.CurrentManager.Busy.creatingInvite)
        {
            App.CurrentManager.CreatePublicInvite();
        }

        Refresh();
        PeerInput.Focus();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => Refresh();

    private void Refresh()
    {
        var invite = App.CurrentManager.PublicInvite;
        var ready = invite != null;
        ReadyBlock.Visibility = ready ? Visibility.Visible : Visibility.Collapsed;
        LoadingText.Visibility = ready ? Visibility.Collapsed : Visibility.Visible;

        if (ready && _qrVisible)
        {
            InviteQr.Text = invite!.url;
            QrPanel.Visibility = Visibility.Visible;
        }
        else if (!ready)
        {
            QrPanel.Visibility = Visibility.Collapsed;
        }
    }

    private void OnCopyInvite(object sender, RoutedEventArgs e)
    {
        var url = App.CurrentManager.PublicInvite?.url;
        if (!string.IsNullOrEmpty(url)) App.CurrentManager.CopyToClipboard(url);
    }

    private void OnShowQr(object sender, RoutedEventArgs e)
    {
        _qrVisible = !_qrVisible;
        Refresh();
    }

    private void OnPeerInputChanged(object sender, TextChangedEventArgs e)
    {
        var raw = PeerInput.Text?.Trim();
        if (string.IsNullOrEmpty(raw)) return;

        var normalized = Native.NormalizePeerInput(raw);
        if (!string.IsNullOrEmpty(normalized) && Native.IsValidPeerInput(normalized))
        {
            if (_lastSubmitted == normalized) return;
            _lastSubmitted = normalized;
            App.CurrentManager.CreateChat(normalized);
            return;
        }

        var lower = raw.ToLowerInvariant();
        if (lower.Contains("://") && lower.Contains("#"))
        {
            if (_lastSubmitted == raw) return;
            _lastSubmitted = raw;
            App.CurrentManager.AcceptInvite(raw);
        }
    }

    private void OnCreateGroup(object sender, RoutedEventArgs e) =>
        App.CurrentManager.Push(new Screen.NewGroup());
}
