using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class JoinInviteView : UserControl
{
    public JoinInviteView()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            UpdateBusy();
            InviteInput.Focus();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => UpdateBusy();

    private void UpdateBusy() =>
        AcceptButton.IsEnabled = !App.CurrentManager.Busy.acceptingInvite;

    private void OnAccept(object sender, RoutedEventArgs e)
    {
        var input = InviteInput.Text?.Trim();
        if (string.IsNullOrEmpty(input)) return;
        App.CurrentManager.AcceptInvite(input);
    }
}
