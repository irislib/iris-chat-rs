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
            SubtitleText.Text = "Use your signed-in device to approve this one.";
            LinkInputBlock.Visibility = Visibility.Collapsed;
            WaitingBlock.Visibility = Visibility.Visible;
        }
        else
        {
            SubtitleText.Text = "Paste the user ID from your signed-in device.";
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
