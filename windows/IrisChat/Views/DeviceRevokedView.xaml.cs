using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class DeviceRevokedView : UserControl
{
    public DeviceRevokedView() { InitializeComponent(); }

    private void OnAcknowledge(object sender, RoutedEventArgs e) =>
        App.CurrentManager.AcknowledgeRevokedDevice();

    private void OnReset(object sender, RoutedEventArgs e) =>
        App.CurrentManager.Logout();
}
