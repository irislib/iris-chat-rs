using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class AddDeviceView : UserControl
{
    private readonly bool _awaitingApproval;

    public AddDeviceView() : this(false) { }

    public AddDeviceView(bool awaitingApproval)
    {
        InitializeComponent();
        _awaitingApproval = awaitingApproval;

        if (_awaitingApproval)
        {
            TitleText.Text = "Finish linking";
            SubtitleText.Text = "Approve the new device from your existing one to complete the handshake.";
            LinkInputBlock.Visibility = Visibility.Collapsed;
            WaitingBlock.Visibility = Visibility.Visible;
        }
        else
        {
            SubtitleText.Text = "Enter the npub of the existing account you want to link this device to.";
        }

        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            UpdateBusy();
            if (!_awaitingApproval) OwnerInput.Focus();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => UpdateBusy();

    private void UpdateBusy()
    {
        if (LinkButton == null) return;
        LinkButton.IsEnabled = !App.CurrentManager.Busy.linkingDevice;
    }

    private void OnLink(object sender, RoutedEventArgs e)
    {
        var input = OwnerInput.Text?.Trim();
        if (string.IsNullOrEmpty(input)) return;
        App.CurrentManager.StartLinkedDevice(input);
    }

    private void OnCancel(object sender, RoutedEventArgs e) => App.CurrentManager.NavigateBack();
}
