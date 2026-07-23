using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class AddDeviceView : UserControl
{
    private bool _attempted;

    public AddDeviceView()
    {
        InitializeComponent();
        SubtitleText.Text = "Scan this code with your signed-in device.";

        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            UpdateBusy();
            if (App.CurrentManager.LinkDevice == null && !App.CurrentManager.Busy.linkingDevice)
            {
                _attempted = true;
                LoadingIndicator.Visibility = Visibility.Visible;
                RetryButton.Visibility = Visibility.Collapsed;
                App.CurrentManager.StartLinkedDevice("");
            }
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => UpdateBusy();

    private void UpdateBusy()
    {
        if (NewCodeButton == null) return;
        var link = App.CurrentManager.LinkDevice;
        var ready = link != null;
        var failed = !ready && !App.CurrentManager.Busy.linkingDevice && _attempted;

        LoadingIndicator.Visibility = !ready && !failed ? Visibility.Visible : Visibility.Collapsed;
        RetryButton.Visibility = failed ? Visibility.Visible : Visibility.Collapsed;
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

    private void OnNewCode(object sender, RoutedEventArgs e)
    {
        _attempted = true;
        RetryButton.Visibility = Visibility.Collapsed;
        LoadingIndicator.Visibility = Visibility.Visible;
        App.CurrentManager.StartLinkedDevice("");
    }

}
