using System;
using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using IrisChat.Bindings;

namespace IrisChat.Views;

public partial class NearbyView : UserControl
{
    private bool _suppressToggleDispatch;

    public NearbyView()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            Refresh();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => Refresh();

    private void Refresh()
    {
        var manager = App.CurrentManager;
        var prefs = manager.Preferences;
        var snapshot = manager.NearbySnapshot;

        _suppressToggleDispatch = true;
        LanToggle.IsChecked = prefs.nearbyLanEnabled;
        _suppressToggleDispatch = false;

        StatusText.Text = snapshot.visible ? WifiStatusLabel(snapshot.status) : "Off";
        RebuildPeers(snapshot.peers ?? Array.Empty<DesktopNearbyPeerSnapshot>());
    }

    private void RebuildPeers(DesktopNearbyPeerSnapshot[] peers)
    {
        PeersList.Items.Clear();
        if (peers.Length == 0)
        {
            PeersList.Items.Add(new Border
            {
                Background = (Brush)FindResource("Panel"),
                CornerRadius = (CornerRadius)FindResource("SectionRadius"),
                Padding = new Thickness(16, 12, 16, 12),
                Child = new TextBlock
                {
                    Text = "No users nearby",
                    Foreground = (Brush)FindResource("TextMuted"),
                },
            });
            return;
        }

        foreach (var peer in peers)
            PeersList.Items.Add(PeerRow(peer));
    }

    private FrameworkElement PeerRow(DesktopNearbyPeerSnapshot peer)
    {
        var name = string.IsNullOrWhiteSpace(peer.name) ? "Iris" : peer.name.Trim();
        var button = new Button
        {
            Style = (Style)FindResource("GhostButton"),
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Padding = new Thickness(0),
            Margin = new Thickness(0, 0, 0, 8),
            IsEnabled = !string.IsNullOrWhiteSpace(peer.ownerPubkeyHex),
        };

        var border = new Border
        {
            Background = (Brush)FindResource("Panel"),
            CornerRadius = (CornerRadius)FindResource("SectionRadius"),
            Padding = new Thickness(16, 12, 16, 12),
            Child = new TextBlock
            {
                Text = name,
                Foreground = (Brush)FindResource("TextPrimary"),
                FontWeight = FontWeights.SemiBold,
            },
        };
        button.Content = border;

        if (!string.IsNullOrWhiteSpace(peer.ownerPubkeyHex))
        {
            var owner = peer.ownerPubkeyHex!;
            button.Click += (_, _) => App.CurrentManager.CreateChat(owner);
        }
        return button;
    }

    private void OnLanChanged(object sender, RoutedEventArgs e)
    {
        if (_suppressToggleDispatch) return;
        App.CurrentManager.SetNearbyLanEnabled(LanToggle.IsChecked == true);
    }

    private static string WifiStatusLabel(string status) =>
        status switch
        {
            "Local network unavailable" => "Wi-Fi unavailable",
            "Local network failed" => "Wi-Fi failed",
            "No local network access" => "No Wi-Fi access",
            _ => status,
        };
}
