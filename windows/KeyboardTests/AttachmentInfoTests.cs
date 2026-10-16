using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Media;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class AttachmentInfoTests
{
    public static void Run(Window owner, AppManager manager)
    {
        var original = manager.State;
        var chat = original.currentChat!;
        var firstUrl = "https://files.example/" + new string('a', 64) + ".bin";
        var secondUrl = "https://backup.example/" + new string('a', 64) + ".bin";
        var attachment = new MessageAttachmentSnapshot("fixture", "photo.jpg", "photo.jpg",
            "htree://fixture/photo.jpg", true, false, false, uploadedUrls: []);
        var message = new ChatMessageSnapshot("attachment-info", chat.chatId, ChatMessageKind.User,
            "You", null, null, "A photo", [attachment], [], [], true, 100, null, DeliveryState.Sent,
            [], new MessageDeliveryTraceSnapshot([], [], [], [], null), null);
        MessageInfoWindow? details = null;
        try
        {
            Apply(message);
            details = new MessageInfoWindow(message) { Owner = owner };
            details.Show(); Pump();
            Check(Links().Length == 0, "A local-only file has no fabricated upload URL");
            var updated = message with { attachments = [attachment with { uploadedUrls = [firstUrl] }] };
            Apply(updated);
            Check(details.IsVisible, "An upload receipt refreshes the open details window");
            var first = Links().Single();
            Check(first.NavigateUri.AbsoluteUri == firstUrl && new TextRange(first.ContentStart, first.ContentEnd).Text == firstUrl,
                "Details keep the full external ciphertext URL visible");
            Check(first.Cursor == System.Windows.Input.Cursors.Hand, "Upload links have a pointer cursor");
            updated = updated with { attachments = [attachment with { uploadedUrls = [firstUrl, secondUrl, "file:///tmp/plaintext", "https://files.example/blob#key"] }] };
            Apply(updated);
            Check(Links().Select(link => link.NavigateUri.AbsoluteUri).SequenceEqual(new[] { firstUrl, secondUrl }),
                "Only safe HTTP blob URLs are clickable");
            Check(ReferenceEquals(first, Links().First()), "Receipt updates preserve the existing link and its interaction state");
            var copy = Descendants<Button>(details).Single(button => Equals(button.Content, "Copy info"));
            copy.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
            Check(Clipboard.GetText().Contains(firstUrl) && Clipboard.GetText().Contains(secondUrl), "Copy info includes full upload URLs");
            var rumor = Descendants<Button>(details).Single(button => Equals(button.Content, "Copy rumor JSON"));
            rumor.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
            Check(!Clipboard.GetText().Contains(firstUrl), "Local upload receipts do not enter rumor JSON");
            Console.WriteLine("PASS: attachment info local-only state, live receipts, full external links, stable controls and copy privacy");
        }
        finally { details?.Close(); ApplyState(original); }

        Hyperlink[] Links() => Descendants<TextBlock>(details!).SelectMany(block => block.Inlines.OfType<Hyperlink>()).ToArray();
        void Apply(ChatMessageSnapshot current) => ApplyState(original with { currentChat = chat with { messages = [current] } });
        void ApplyState(AppState state)
        {
            const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic;
            typeof(AppManager).GetField("_state", flags)!.SetValue(manager, state);
            typeof(AppManager).GetMethod("Notify", flags)!.Invoke(manager, new object[] { nameof(AppManager.CurrentChat) });
            Pump();
        }
    }

    private static IEnumerable<T> Descendants<T>(DependencyObject parent) where T : DependencyObject
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); index++)
        {
            var child = VisualTreeHelper.GetChild(parent, index);
            if (child is T match) yield return match;
            foreach (var descendant in Descendants<T>(child)) yield return descendant;
        }
    }
    private static void Pump()
    {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }
    private static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
}
