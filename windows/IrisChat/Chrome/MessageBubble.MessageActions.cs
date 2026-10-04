using System;
using System.ComponentModel;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

public partial class MessageBubble
{
    public event Action<ChatMessageSnapshot>? EditRequested;

    internal static bool CanDeleteForEveryone(ChatMessageSnapshot message) =>
        message.isOutgoing && message.kind == ChatMessageKind.User && !message.deletedForEveryone &&
        message.call == null && message.directTransfer == null &&
        message.delivery is DeliveryState.Sent or DeliveryState.Received or DeliveryState.Seen;

    internal static bool CanEdit(ChatMessageSnapshot message) => CanDeleteForEveryone(message) &&
        !string.IsNullOrWhiteSpace(message.body) && message.attachments.Length == 0;

    private void ConfirmDeleteForEveryone(ChatMessageSnapshot message)
    {
        if (MessageBox.Show(Window.GetWindow(this),
            "Ask everyone to delete this message and its edit history?",
            "Delete for everyone", MessageBoxButton.OKCancel, MessageBoxImage.Warning,
            MessageBoxResult.Cancel) == MessageBoxResult.OK)
            App.CurrentManager.DeleteMessageForEveryone(message.chatId, message.id);
    }

    private void OnShowEditHistory(object sender, RoutedEventArgs e)
    {
        if (_message is not { } target || target.deletedForEveryone || target.editHistory is not { Length: > 0 }) return;
        var window = new Window
        {
            Title = "Edit history", Width = 440, Height = 520,
            Owner = Window.GetWindow(this), WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ShowInTaskbar = false, Background = (Brush)FindResource("Background"),
        };
        var versions = new StackPanel { Margin = new Thickness(20) };
        window.Content = new ScrollViewer
        {
            Content = versions, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        };
        ChatMessageSnapshot? rendered = null;
        void RefreshHistory()
        {
            var chat = App.CurrentManager.CurrentChat;
            var message = chat?.chatId == target.chatId ? chat.messages.FirstOrDefault(m => m.id == target.id) : null;
            if (message == null || message.deletedForEveryone || message.editHistory is not { Length: > 0 })
            {
                window.Close();
                return;
            }
            if (rendered?.editHistory != null && rendered.editHistory.SequenceEqual(message.editHistory)) return;
            rendered = message;
            versions.Children.Clear();
            for (var i = 0; i < message.editHistory.Length; i++)
            {
                var version = message.editHistory[i];
                var label = i == 0 ? "Original" : i == message.editHistory.Length - 1 ? "Current" : $"Edit {i}";
                versions.Children.Add(new TextBlock
                {
                    Text = label, FontWeight = FontWeights.SemiBold,
                    Foreground = (Brush)FindResource("TextPrimary"), Margin = new Thickness(0, i == 0 ? 0 : 20, 0, 4),
                });
                versions.Children.Add(new TextBlock
                {
                    Text = DateTimeOffset.FromUnixTimeSeconds((long)Math.Min(version.createdAtSecs, 253402300799UL)).LocalDateTime.ToString("g"),
                    Foreground = (Brush)FindResource("TextMuted"), FontSize = 11, Margin = new Thickness(0, 0, 0, 6),
                });
                versions.Children.Add(new TextBox
                {
                    Text = version.body, IsReadOnly = true, TextWrapping = TextWrapping.Wrap,
                    BorderThickness = new Thickness(0), Background = Brushes.Transparent,
                    Foreground = (Brush)FindResource("TextPrimary"), Padding = new Thickness(0),
                });
            }
        }
        PropertyChangedEventHandler changed = (_, _) => RefreshHistory();
        App.CurrentManager.PropertyChanged += changed;
        window.Closed += (_, _) => App.CurrentManager.PropertyChanged -= changed;
        window.Loaded += (_, _) => RefreshHistory();
        window.ShowDialog();
    }
}
