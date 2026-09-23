using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using IrisChat.Bindings;

namespace IrisChat.Views;

public partial class RemoteSignerView : UserControl
{
    private bool _showingLinkInput;

    public RemoteSignerView()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            UpdateState();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => UpdateState();

    private void UpdateState()
    {
        var login = App.CurrentManager.State.remoteSignerLogin;
        var awaitingApproval = login?.phase is RemoteSignerPhase.WaitingForApproval or RemoteSignerPhase.Finishing;
        var hasCode = !string.IsNullOrEmpty(login?.connectionUri) && !awaitingApproval;
        StatusText.Text = hasCode ? "Scan with your signer app." : login?.phase switch
        {
            RemoteSignerPhase.Connecting => "Connecting…",
            RemoteSignerPhase.WaitingForSigner => "Waiting for your signer…",
            RemoteSignerPhase.WaitingForApproval => "Approve in your signer app.",
            RemoteSignerPhase.Finishing => "Signing in…",
            _ => "",
        };
        StatusText.Visibility = login == null ? Visibility.Collapsed : Visibility.Visible;
        LoadingIndicator.Visibility = login != null && !hasCode ? Visibility.Visible : Visibility.Collapsed;
        SignerCode.Visibility = hasCode ? Visibility.Visible : Visibility.Collapsed;
        CopyButton.Visibility = hasCode ? Visibility.Visible : Visibility.Collapsed;
        RetryButton.Visibility = login == null ? Visibility.Visible : Visibility.Collapsed;
        ApprovalButton.Visibility = ApprovalUri() != null ? Visibility.Visible : Visibility.Collapsed;
        PasteButton.Visibility = !awaitingApproval && !_showingLinkInput ? Visibility.Visible : Visibility.Collapsed;
        LinkInputBlock.Visibility = _showingLinkInput && !awaitingApproval ? Visibility.Visible : Visibility.Collapsed;
        if (hasCode) SignerCode.Text = login!.connectionUri;
    }

    private static Uri? ApprovalUri()
    {
        var value = App.CurrentManager.State.remoteSignerLogin?.authUrl;
        return Uri.TryCreate(value, UriKind.Absolute, out var uri) &&
            (uri.Scheme == "https" || uri.Scheme == "http") && !string.IsNullOrEmpty(uri.Host) ? uri : null;
    }

    private void OnCopy(object sender, RoutedEventArgs e)
    {
        var code = App.CurrentManager.State.remoteSignerLogin?.connectionUri;
        if (!string.IsNullOrEmpty(code)) App.CurrentManager.CopyToClipboard(code);
    }

    private void OnRetry(object sender, RoutedEventArgs e) => App.CurrentManager.StartRemoteSignerLogin();

    private void OnApproval(object sender, RoutedEventArgs e)
    {
        if (ApprovalUri() is { } uri)
        {
            try { Process.Start(new ProcessStartInfo(uri.AbsoluteUri) { UseShellExecute = true }); }
            catch (System.ComponentModel.Win32Exception) { }
        }
    }

    private void OnPaste(object sender, RoutedEventArgs e)
    {
        _showingLinkInput = true;
        LinkInput.Text = PlatformClipboard.GetString() ?? "";
        UpdateState();
        LinkInput.Focus();
    }

    private void OnLinkChanged(object sender, TextChangedEventArgs e)
    {
        if (ConnectButton != null) ConnectButton.IsEnabled = !string.IsNullOrWhiteSpace(LinkInput.Text);
    }

    private void OnConnect(object sender, RoutedEventArgs e)
    {
        var value = LinkInput.Text;
        _showingLinkInput = false;
        LinkInput.Clear();
        App.CurrentManager.ConnectRemoteSigner(value);
        UpdateState();
    }
}
