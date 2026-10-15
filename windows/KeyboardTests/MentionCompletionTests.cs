using System;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Input;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class MentionCompletionTests
{
    public static void Run(Window window)
    {
        Check(ComposerMentionCompletion.Find("@", 1, 0)?.query == "", "Bare @ opens immediately");
        Check(ComposerMentionCompletion.Find("person@example", 14, 0) == null, "Email stays literal");
        Check(ComposerMentionCompletion.Find("@Alex", 3, 1) == null, "Selection cannot retarget a mention");
        var first = new string('1', 64); var second = new string('2', 64);
        var token = Native.FormatMessageMention(first, "Old name");
        var model = new MentionDraft();
        var visible = model.Restore("👋 " + token);
        var modelInput = new TextBox { Text = visible };
        modelInput.TextChanged += (_, e) => model.Changed(modelInput.Text, e.Changes, e.UndoAction, modelInput.UndoLimit);
        modelInput.Select(0, 0); modelInput.SelectedText = "Hello ";
        Check(model.WireText(modelInput.Text) == "Hello 👋 " + token, "Unicode edits preserve exact identity");
        modelInput.Select("Hello 👋 ".Length, modelInput.Text.Length - "Hello 👋 ".Length);
        modelInput.SelectedText = "@Someone else";
        Check(Native.MessageMentions(model.WireText("Hello 👋 @Someone else")).Length == 0, "Edited labels never guess identity");
        var previous = window.Content;
        var composer = new ComposerBar();
        composer.MentionParticipants = new[] {
            new ChatParticipantSnapshot(null, first, "Alex", null, false),
            new ChatParticipantSnapshot(null, second, "Alex", null, false)
        };
        window.Content = composer;
        try
        {
            var sent = 0; string saved = "";
            composer.Submitted += (_, _) => sent++;
            composer.DraftChanged += text => saved = text;
            Pump();
            window.Activate();
            composer.FocusInput(); Pump();
            var input = (TextBox)composer.FindName("Input");
            Check(input.IsKeyboardFocused, "Loaded mention composer receives keyboard focus");
            input.AppendText("Hi @"); input.CaretIndex = input.Text.Length; Pump();
            Check(Press(input, Key.Down).Handled, "Keyboard chooses between same-name people");
            Check(Press(input, Key.Enter).Handled && sent == 0, "Accepting mention does not send");
            Check(Native.MessageMentions(saved).Single().ownerPubkeyHex == second, "Selection saves chosen identity");
            Check(input.Text == "Hi @Alex ", "Composer displays readable name");
            var savedMention = saved;
            input.Undo(); Pump();
            Check(input.Text == "Hi @" && !Press(input, Key.Down).Handled, "Undo restores text without reopening menu");
            input.Redo(); Pump();
            Check(input.Text == "Hi @Alex ", "Redo restores the visible mention");
            MentionEditingTests.Identities(saved, second);
            composer.RestoreDraft(savedMention); Pump();
            Check(input.Text == "Hi @Alex ", "Saved wire restores readable identity");
            Check(!Press(input, Key.Down).Handled, "Restored draft does not open menu");
            composer.RestoreDraft("");
            input.AppendText("@"); input.CaretIndex = 1; Pump();
            Check(Press(input, Key.Escape).Handled && input.Text == "@", "Dismissal preserves typing");
            composer.RestoreDraft("");
            input.AppendText("@"); input.CaretIndex = 1;
            Check(Press(input, Key.Enter, pump: false).Handled && sent == 0,
                "Enter resolves pending mention completion before sending, without a dispatcher drain");
            MentionEditingTests.Identities(saved, first);
            Pump();
            MentionEditingTests.Run(composer, input, first, second, () => saved, Pump);

            var body = new RichTextBox { IsReadOnly = true, IsDocumentEnabled = true };
            window.Content = body;
            MentionText.Render(body, "Hello " + token + " https://iris.to, www.example.org!"); Pump();
            var paragraph = body.Document.Blocks.OfType<Paragraph>().Single();
            var links = paragraph.Inlines.OfType<Hyperlink>().ToArray();
            Check(links.Length == 3 && links.All(link => link.Cursor == Cursors.Hand), "Only actionable profile and URL spans use hand cursors");
            var text = new TextRange(body.Document.ContentStart, body.Document.ContentEnd).Text.TrimEnd();
            Check(text == "Hello @Old name https://iris.to, www.example.org!", "Native selectable text preserves punctuation and visible labels");
            body.SelectAll(); body.Copy();
            Check(Clipboard.GetText().TrimEnd() == text, "Native copy exposes readable selected text");
            Check(body.IsTabStop && links.All(link => link.IsEnabled), "Links remain keyboard accessible");
        }
        finally { window.Content = previous; }
        Console.WriteLine("PASS: mention identity, keyboard selection, dismissal, selectable links and copy");
    }
    private static KeyEventArgs Press(TextBox input, Key key, bool pump = true)
    {
        var args = new KeyEventArgs(Keyboard.PrimaryDevice, PresentationSource.FromVisual(input), 0, key) { RoutedEvent = Keyboard.PreviewKeyDownEvent };
        input.RaiseEvent(args); if (pump) Pump(); return args;
    }
    private static void Pump()
    {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }
    private static void Check(bool passed, string message) { if (!passed) throw new Exception(message); }
}
