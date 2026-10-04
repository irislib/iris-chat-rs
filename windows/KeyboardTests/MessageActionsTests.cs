using System;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class MessageActionsTests
{
    public static void Run(Window window)
    {
        var composer = new ComposerBar();
        window.Content = composer;
        var input = (TextBox)composer.FindName("Input");
        var send = (Button)composer.FindName("SendButton");
        input.Text = "Unsent draft";
        composer.AddAttachments(new[] { "draft.txt" });
        string? savedId = null;
        string? savedBody = null;
        composer.EditSubmitted += (id, body) => { savedId = id; savedBody = body; };
        composer.BeginEdit("original", "Before");
        Check(input.Text == "Before" && composer.EditingMessageId == "original", "Edit loads original text");
        Check(!((Button)composer.FindName("AttachButton")).IsEnabled, "Editing disables attachments");
        composer.AddAttachments(new[] { "not-an-edit.txt" });
        Check(composer.StagedFilePaths.SequenceEqual(new[] { "draft.txt" }), "Editing preserves the draft's attachments");
        input.Text = "   ";
        send.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
        Check(savedId == null && composer.EditingMessageId == "original", "Blank edits remain in composer");
        input.Text = "After";
        send.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
        Check(savedId == "original" && savedBody == "After", "Save submits the edit target and new text");
        Check(input.Text == "Unsent draft" && composer.EditingMessageId == null, "Save restores the unsent draft");
        Check(composer.StagedFilePaths.SequenceEqual(new[] { "draft.txt" }), "Save retains staged draft files");
        composer.BeginEdit("original", "After");
        input.Text = "Discard this change";
        composer.CancelEdit();
        Check(input.Text == "Unsent draft" && savedBody == "After", "Cancel restores draft without saving");

        var message = new ChatMessageSnapshot("message", "chat", ChatMessageKind.User,
            "You", null, null, "Current body", [], [], [], true, 100, null, DeliveryState.Sent,
            [], new MessageDeliveryTraceSnapshot([], [], [], [], null), null,
            editHistory: [new MessageEditSnapshot("message", "Original body", 100),
                new MessageEditSnapshot("edit", "Current body", 101)]);
        var bubble = new MessageBubble();
        bubble.Bind(message, showFooter: false);
        Check(((Button)bubble.FindName("EditedButton")).Visibility == Visibility.Visible,
            "Edited is visible even in a grouped message");
        Check(bubble.ContextMenu.Items.OfType<MenuItem>().Any(item => Equals(item.Header, "Edit message")), "Own text offers editing");
        bubble.Bind(message with { deletedForEveryone = true, body = "", editHistory = [] });
        Check(((TextBox)bubble.FindName("BodyText")).Text == "Message deleted", "Deleted content is a tombstone");
        Check(((Button)bubble.FindName("EditedButton")).Visibility == Visibility.Collapsed, "Deleted message has no history link");
        Check(!bubble.ContextMenu.Items.OfType<MenuItem>().Any(item =>
            Equals(item.Header, "Edit message") || Equals(item.Header, "React") || Equals(item.Header, "Edit history")),
            "Deleted messages cannot be edited, reacted to, or expose history");
        composer.Clear();
        Console.WriteLine("PASS: message edit Save/Cancel, draft and attachment preservation, history and deleted-message controls");
    }

    private static void Check(bool value, string context) { if (!value) throw new Exception(context); }
}
