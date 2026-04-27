using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class CreateInviteView : UserControl
{
    public CreateInviteView()
    {
        InitializeComponent();
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        App.CurrentManager.PropertyChanged += OnChanged;
        if (App.CurrentManager.PublicInvite == null && !App.CurrentManager.Busy.creatingInvite)
        {
            App.CurrentManager.CreatePublicInvite();
        }
        Refresh();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e) =>
        App.CurrentManager.PropertyChanged -= OnChanged;

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => Refresh();

    private void Refresh()
    {
        var invite = App.CurrentManager.PublicInvite;
        var ready = invite != null;

        LoadingText.Visibility = ready ? Visibility.Collapsed : Visibility.Visible;
        InviteQr.Visibility = ready ? Visibility.Visible : Visibility.Collapsed;
        ButtonsRow.Visibility = ready ? Visibility.Visible : Visibility.Collapsed;

        if (ready) InviteQr.Text = invite!.url;

        NewInviteButton.IsEnabled = !App.CurrentManager.Busy.creatingInvite;
    }

    private void OnCopy(object sender, RoutedEventArgs e)
    {
        var url = App.CurrentManager.PublicInvite?.url;
        if (!string.IsNullOrEmpty(url)) App.CurrentManager.CopyToClipboard(url);
    }

    private void OnNewInvite(object sender, RoutedEventArgs e) =>
        App.CurrentManager.CreatePublicInvite();
}
