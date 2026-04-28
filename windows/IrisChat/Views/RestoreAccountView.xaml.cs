using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class RestoreAccountView : UserControl
{
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
        RestoreButton.IsEnabled = !App.CurrentManager.Busy.restoringSession;

    private void OnRestore(object sender, RoutedEventArgs e)
    {
        var nsec = NsecInput.Password?.Trim();
        if (string.IsNullOrEmpty(nsec)) return;
        App.CurrentManager.RestoreSession(nsec);
    }
}
