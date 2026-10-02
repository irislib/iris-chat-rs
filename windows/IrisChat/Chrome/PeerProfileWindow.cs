using System;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

public static class PeerProfileWindow
{
    public static void Show(
        Window? owner,
        AppManager manager,
        DesktopNearbyPeerSnapshot peer,
        Action<string>? onMessage = null)
    {
        if (string.IsNullOrWhiteSpace(peer.ownerPubkeyHex)) return;

        ShowProfile(owner, manager, peer.ownerPubkeyHex!,
            NearbyPeerNames.Resolve(manager, peer), peer.pictureUrl, onMessage);
    }

    public static void ShowProfile(
        Window? owner, AppManager manager, string ownerPubkeyHex,
        string fallbackName, string? fallbackPictureUrl = null,
        Action<string>? onMessage = null)
    {
        if (string.IsNullOrWhiteSpace(ownerPubkeyHex)) return;
        var chat = manager.ChatList.FirstOrDefault(c => c.kind == ChatKind.Direct && c.chatId == ownerPubkeyHex);
        var person = manager.CurrentChat?.participants.FirstOrDefault(p => p.ownerPubkeyHex == ownerPubkeyHex);
        var displayName = chat?.displayName ?? person?.displayName ?? fallbackName;
        var pictureUrl = chat?.pictureUrl ?? person?.pictureUrl ?? fallbackPictureUrl;
        var window = new Window
        {
            Title = displayName,
            Width = 380,
            Height = 360,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ShowInTaskbar = false,
            ResizeMode = ResizeMode.CanResize,
            Owner = owner,
            Background = Brush("Background"),
        };

        var stack = new StackPanel
        {
            Orientation = Orientation.Vertical,
            Margin = new Thickness(20, 18, 20, 18),
        };

        var header = new Grid { Margin = new Thickness(0, 0, 0, 14) };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.Children.Add(new Avatar
        {
            Label = displayName,
            OwnerPubkeyHex = ownerPubkeyHex,
            PictureUrl = pictureUrl,
            Size = 64,
            Margin = new Thickness(0, 0, 14, 0),
        });
        var name = new TextBlock
        {
            Text = displayName,
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            Foreground = Brush("TextPrimary"),
            TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center,
        };
        Grid.SetColumn(name, 1);
        header.Children.Add(name);
        stack.Children.Add(header);

        var message = new Button
        {
            Content = "Message",
            Padding = new Thickness(12, 7, 12, 7),
            HorizontalAlignment = HorizontalAlignment.Left,
            Margin = new Thickness(0, 4, 0, 0),
        };
        message.Click += (_, _) =>
        {
            if (onMessage is not null)
                onMessage(ownerPubkeyHex);
            else
                manager.OpenChat(ownerPubkeyHex);
            window.Close();
        };
        stack.Children.Add(message);

        window.Content = stack;
        window.ShowDialog();
    }

    private static Brush Brush(string key) => (Brush)Application.Current.Resources[key];
}
