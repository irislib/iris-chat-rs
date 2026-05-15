using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class DeviceRevokedView : UserControl
{
    public DeviceRevokedView() { InitializeComponent(); }

    private void OnAcknowledge(object sender, RoutedEventArgs e) =>
        ConfirmLogout();

    private static void ConfirmLogout()
    {
        var result = MessageBox.Show(
            "This removes your secret keys, messages, and cached files from this device.",
            "Delete all local data?",
            MessageBoxButton.OKCancel,
            MessageBoxImage.Warning
        );
        if (result == MessageBoxResult.OK)
            App.CurrentManager.Logout();
    }
}
