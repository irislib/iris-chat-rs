using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class DeviceRevokedView : UserControl
{
    public DeviceRevokedView() { InitializeComponent(); }

    private void OnAcknowledge(object sender, RoutedEventArgs e) =>
        App.CurrentManager.AcknowledgeRevokedDevice();

    private void OnReset(object sender, RoutedEventArgs e)
    {
        var result = MessageBox.Show(
            Window.GetWindow(this),
            "This removes your secret keys, messages, and cached files from this device.",
            "Delete app data?",
            MessageBoxButton.OKCancel,
            MessageBoxImage.Warning
        );
        if (result == MessageBoxResult.OK)
            App.CurrentManager.Logout();
    }
}
