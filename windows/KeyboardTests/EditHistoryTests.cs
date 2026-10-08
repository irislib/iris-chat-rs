using System;
using System.Linq;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class EditHistoryTests
{
    public static void Run(Window owner, AppManager manager)
    {
        var original = manager.State;
        var chat = original.currentChat!;
        var message = new ChatMessageSnapshot("history-target", chat.chatId, ChatMessageKind.User,
            "Alex", null, null, "Current body", [], [], [], false, 100, null, DeliveryState.Received,
            [], new MessageDeliveryTraceSnapshot([], [], [], [], null), null,
            editHistory: [new MessageEditSnapshot("original", "Original body", 100),
                new MessageEditSnapshot("edit-1", "↩ Sam: Earlier message\n\nFirst edit", 101),
                new MessageEditSnapshot("edit-2", "Current body", 102)]);
        var state = original with { currentChat = chat with { messages = [message] } };
        try
        {
            Apply(state);
            var history = Open();
            var versions = (StackPanel)((ScrollViewer)history.Content).Content;
            Check(versions.Children.OfType<TextBox>().Select(box => box.Text)
                .SequenceEqual(new[] { "Current body", "First edit", "Original body" }), "History is newest first");
            Check(versions.Children.OfType<TextBlock>().Where((_, i) => i % 2 == 0).Select(label => label.Text)
                .SequenceEqual(new[] { "Current", "Edit 1", "Original" }), "History labels identify each revision");
            Check(versions.Children.OfType<TextBox>().All(box => box.IsReadOnly && box.IsEnabled), "History text stays selectable");
            Check(MessageEditHistoryWindow.ReadableBody("↩ malformed\n\nKeep the text") == "↩ malformed\n\nKeep the text", "Invalid reply encoding remains visible");
            var latest = message with { body = "Live edit", editHistory = message.editHistory.Append(new MessageEditSnapshot("edit-3", "Live edit", 103)).ToArray() };
            Apply(state with { currentChat = chat with { messages = [latest] } });
            Check(versions.Children.OfType<TextBox>().First().Text == "Live edit", "Open history follows edits");
            history.Close();

            Apply(state);
            var details = new MessageInfoWindow(message) { Owner = owner };
            details.Show(); Pump();
            var content = (StackPanel)((ScrollViewer)details.Content).Content;
            var action = content.Children.OfType<Button>().Single(button => Equals(button.Content, "Edit history"));
            var openedFromDetails = false;
            owner.Dispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(() =>
            {
                var shown = Application.Current.Windows.OfType<MessageEditHistoryWindow>().Single();
                openedFromDetails = shown.IsVisible;
                shown.Close();
            }));
            action.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
            Check(openedFromDetails && !details.IsVisible, "Message Details opens the same history window");
            details = new MessageInfoWindow(message) { Owner = owner };
            details.Show(); Pump();
            Apply(state with { account = null });
            Check(!details.IsVisible, "Message Details cannot retain a history link across accounts");

            foreach (var invalid in new[] {
                state with { currentChat = chat with { messages = [] } },
                state with { currentChat = chat with { messages = [message with { deletedForEveryone = true, editHistory = [] }] } },
                state with { currentChat = chat with { messages = [message with { expiresAtSecs = 1 }] } },
                state with { currentChat = chat with { chatId = "different-chat" } },
                state with { account = null },
                state with { router = state.router with { screenStack = [new Screen.Settings()] } },
            })
            {
                Apply(state);
                history = Open();
                Apply(invalid);
                Check(!history.IsVisible, "Invalid or inactive message history closes");
            }
            Console.WriteLine("PASS: Windows edit history menu/details, current-first selectable versions, live edits and dismissal guards");
        }
        finally { Apply(original); }

        MessageEditHistoryWindow Open()
        {
            var history = new MessageEditHistoryWindow(message) { Owner = owner };
            history.Show(); Pump();
            Check(history.IsVisible, "History opens for the active message");
            return history;
        }
        void Apply(AppState next)
        {
            const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic;
            typeof(AppManager).GetField("_state", flags)!.SetValue(manager, next);
            typeof(AppManager).GetMethod("Notify", flags)!.Invoke(manager, new object[] { nameof(AppManager.CurrentChat) });
            Pump();
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
