using System;
using System.Linq;
using System.Reflection;
using System.Threading;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;
using IrisChat.Views;

internal static class DraftTests
{
    public static void Run(Window window, AppManager manager)
    {
        var other = manager.CurrentChat!.chatId;
        var own = manager.Account!.publicKeyHex;
        manager.CreateChat(own);
        Until(() => manager.CurrentChat?.chatId == own);
        var view = new ChatView();
        window.Content = view;
        Pump();
        var composer = (ComposerBar)view.FindName("Composer");
        var input = (TextBox)composer.FindName("Input");
        string Draft(string id) => manager.ChatList.First(chat => chat.chatId == id).draft;

        int draftEvents = 0, typingEvents = 0, stoppedEvents = 0;
        composer.DraftChanged += _ => draftEvents++;
        composer.Typing += () => typingEvents++;
        composer.StoppedTyping += () => stoppedEvents++;
        composer.RestoreDraft("Hydrated text");
        composer.RestoreDraft("");
        Check(draftEvents == 0 && typingEvents == 0 && stoppedEvents == 0,
            "Draft hydration and programmatic clear never broadcast typing or draft changes");

        const string firstDraft = "  Forest walk\nBring a map  ";
        input.Text = firstDraft;
        Check(draftEvents == 1 && typingEvents == 1 && stoppedEvents == 0,
            "A genuine user edit still saves the draft and starts typing");
        manager.OpenChat(other); // Switch before the 500 ms save timer fires.
        Until(() => manager.CurrentChat?.chatId == other && input.Text.Length == 0);
        const string otherDraft = "  Other local plan\n ";
        input.Text = otherDraft;
        manager.SetChatPinned(other, true); // A state refresh still carries the older stored draft.
        Until(() => manager.ChatList.First(chat => chat.chatId == other).isPinned);
        Check(input.Text == otherDraft, "State refresh cannot overwrite the new chat's local typing");
        Until(() => Draft(own) == firstDraft && Draft(other) == otherDraft);
        manager.OpenChat(own);
        Until(() => manager.CurrentChat?.chatId == own && input.Text == firstDraft);
        Check(Draft(other) == otherDraft, "Pending A save never enters B");

        var draftEventsBeforeEdit = draftEvents;
        composer.BeginEdit("edit-fixture", "Previously sent text");
        input.Text = "Changes to the sent message";
        DrainDraftTimer();
        Check(Draft(own) == firstDraft, "Edit text cannot become the ordinary chat draft");
        composer.CancelEdit();
        Check(input.Text == firstDraft, "Cancel edit restores the exact unsent draft");
        Check(draftEvents == draftEventsBeforeEdit, "Edit loading, typing and cancellation never replace the ordinary draft");

        input.Text = " \n\t ";
        Until(() => Draft(own) == input.Text);
        var row = new ChatRow { Chat = manager.ChatList.First(chat => chat.chatId == own) with
            { lastMessagePreview = "Earlier message" } };
        var preview = (TextBlock)row.FindName("PreviewText");
        Check(preview.Text == "Earlier message", "Whitespace-only drafts use the last message preview");
        row.Chat = row.Chat! with { draft = firstDraft };
        Check(preview.Text == "Draft: Forest walk\nBring a map", "List trims the preview while storage retains the full draft");
        Check(preview.Inlines.OfType<Run>().First().FontStyle == FontStyles.Italic, "Draft prefix is distinguished from message text");
        row.Chat = row.Chat! with { isTyping = true };
        Check(preview.Text == "typing…", "Typing takes precedence over the saved draft");
        row.Chat = row.Chat! with { isTyping = false, draft = "" };
        Check(preview.Text == "Earlier message", "Cleared drafts return to the message preview");
        input.Clear();
        Until(() => Draft(own).Length == 0);

        Until(() => composer.SendAllowed);
        input.Text = "Send the pending plan";
        ((Button)composer.FindName("SendButton")).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
        Until(() => manager.CurrentChat?.messages.Any(message => message.body == "Send the pending plan") == true
            && Draft(own).Length == 0);
        DrainDraftTimer();
        // Existing FIFO query is a barrier through the real production Core, not a projected test state.
        var ffi = (FfiApp)typeof(AppManager).GetField("_ffi", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(manager)!;
        _ = ffi.ExportSupportBundleJson();
        Check(ffi.State().chatList.First(chat => chat.chatId == own).draft.Length == 0,
            "Pending draft save precedes send and cannot resurrect the cleared draft");
        var sentId = manager.CurrentChat!.messages.Single(message => message.body == "Send the pending plan").id;
        manager.DeleteLocalMessage(own, sentId);
        Until(() => manager.CurrentChat?.messages.Any(message => message.id == sentId) == false);

        const string unloadDraft = "  Keep this on leaving\n ";
        input.Text = unloadDraft;
        window.Content = new TextBlock { Text = "Chats" };
        Pump();
        Until(() => Draft(own) == unloadDraft);
        manager.OpenChat(other);
        Until(() => manager.CurrentChat?.chatId == other);
        manager.OpenChat(own);
        Until(() => manager.CurrentChat?.chatId == own);
        view = new ChatView();
        window.Content = view;
        Pump();
        input = (TextBox)((ComposerBar)view.FindName("Composer")).FindName("Input");
        Check(input.Text == unloadDraft, "Unload flush and fresh view restore preserve exact draft text");
        input.Clear();
        window.Content = new TextBlock { Text = "Chats" };
        Pump();
        Until(() => Draft(own).Length == 0);

        var closingView = new ChatView();
        var secondary = new Window { Width = 420, Height = 300, Content = closingView };
        secondary.Show();
        Pump();
        var closingInput = (TextBox)((ComposerBar)closingView.FindName("Composer")).FindName("Input");
        const string closingDraft = "  Save before window closes\n ";
        closingInput.Text = closingDraft;
        bool savedDuringClosing = false;
        secondary.Closing += (_, _) =>
        {
            // Added after Loaded: verify the production Closing flush before Unloaded can run.
            _ = ffi.ExportSupportBundleJson();
            savedDuringClosing = ffi.State().chatList.First(chat => chat.chatId == own).draft == closingDraft;
        };
        secondary.Close();
        Check(savedDuringClosing, "Window.Closing saves the pending draft before unload or app shutdown");
        Until(() => Draft(own) == closingDraft);
        manager.SetChatDraft(own, "");
        Until(() => Draft(own).Length == 0);
        Console.WriteLine("PASS: real Core drafts, pending chat switch, edit isolation, clear/send, unload/close/restore and list precedence");
    }

    private static void DrainDraftTimer()
    {
        var deadline = DateTime.UtcNow.AddMilliseconds(650);
        do { Pump(); Thread.Sleep(2); } while (DateTime.UtcNow < deadline);
    }

    private static void Until(Func<bool> condition)
    {
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (!condition()) { Check(DateTime.UtcNow < deadline, "Draft fixture timed out"); Pump(); Thread.Sleep(2); }
    }

    private static void Pump()
    {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }

    private static void Check(bool value, string context) { if (!value) throw new Exception(context); }
}
