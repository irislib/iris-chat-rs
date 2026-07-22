using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class AddDeviceView : UserControl
{
    public AddDeviceView()
    {
        InitializeComponent();
        SubtitleText.Text = "Scan this code with your signed-in device.";

        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            if (App.CurrentManager.LinkDevice == null && !App.CurrentManager.Busy.linkingDevice)
            {
                App.CurrentManager.StartLinkedDevice("");
            }
            UpdateBusy();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => UpdateBusy();

    private void UpdateBusy()
    {
        if (NewCodeButton == null) return;
        var link = App.CurrentManager.LinkDevice;
        var ready = link != null;

        LoadingText.Visibility = ready ? Visibility.Collapsed : Visibility.Visible;
        LinkQr.Visibility = ready ? Visibility.Visible : Visibility.Collapsed;
        LinkButtons.Visibility = ready ? Visibility.Visible : Visibility.Collapsed;
        NewCodeButton.IsEnabled = !App.CurrentManager.Busy.linkingDevice;

        if (ready) LinkQr.Text = link!.url;
    }

    private void OnCopy(object sender, RoutedEventArgs e)
    {
        var url = App.CurrentManager.LinkDevice?.url;
        if (!string.IsNullOrEmpty(url)) App.CurrentManager.CopyToClipboard(url);
    }

    private void OnNewCode(object sender, RoutedEventArgs e) =>
        App.CurrentManager.StartLinkedDevice("");

}
