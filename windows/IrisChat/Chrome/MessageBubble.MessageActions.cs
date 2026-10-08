using System;
using System.Windows;
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
        if (_message is not { } target || !MessageEditHistoryWindow.CanShow(target)) return;
        new MessageEditHistoryWindow(target) { Owner = Window.GetWindow(this) }.ShowDialog();
    }
}
