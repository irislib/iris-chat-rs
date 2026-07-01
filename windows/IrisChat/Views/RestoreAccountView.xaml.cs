using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class RestoreAccountView : UserControl
{
    private string? _lastSubmittedSecret;

    public RestoreAccountView()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            UpdateBusy();
            NsecInput.Focus();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => UpdateBusy();

    private void UpdateBusy() =>
        NsecInput.IsEnabled = !App.CurrentManager.Busy.restoringSession;

    private void OnSecretChanged(object sender, RoutedEventArgs e)
    {
        if (App.CurrentManager.Busy.restoringSession) return;
        var current = NsecInput.Password?.Trim() ?? "";
        if (!ShouldAutoSubmitSecret(current)) return;
        if (_lastSubmittedSecret == current) return;
        _lastSubmittedSecret = current;
        NsecInput.IsEnabled = false;
        App.CurrentManager.RestoreSession(current);
    }

    private static bool ShouldAutoSubmitSecret(string current)
    {
        if (string.IsNullOrEmpty(current)) return false;
        var lower = current.ToLowerInvariant();
        if (lower.StartsWith("nsec1"))
        {
            return current.Length >= 63;
        }
        if (current.Length != 64) return false;
        foreach (var ch in current)
        {
            if (!IsAsciiHexDigit(ch)) return false;
        }
        return true;
    }

    private static bool IsAsciiHexDigit(char ch) =>
        (ch >= '0' && ch <= '9') ||
        (ch >= 'a' && ch <= 'f') ||
        (ch >= 'A' && ch <= 'F');
}
