using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Chrome;

internal static class EmojiCompletionTests
{
    public static void Run(Window window)
    {
        foreach (var text in new[] { "12:30", "https://iris.to/:sm", "word:sm", ":smile::", new string(':', 100_000) })
            Check(EmojiCompletionToken.Find(text, text.Length, 0) == null, "Literal colon content is untouched");
        var token = EmojiCompletionToken.Find("👩🏽‍💻 :smile: after", "👩🏽‍💻 :sm".Length, 0)!;
        Check("👩🏽‍💻 :smile: after".Remove(token.Start, token.Length).Insert(token.Start, "😄") == "👩🏽‍💻 😄 after", "UTF16 range preserves surrounding text");
        var previous = window.Content;
        var composer = new ComposerBar();
        window.Content = composer;
        try
        {
            Pump();
            window.Activate();
            composer.FocusInput();
            Pump();
            var input = (TextBox)composer.FindName("Input");
            Check(input.IsKeyboardFocused, "Loaded emoji composer receives keyboard focus");
            var sent = 0;
            composer.Submitted += (_, _) => sent++;
            input.AppendText("hi :sm"); input.CaretIndex = input.Text.Length; Pump();
            CompletionScreenshots.Menu(composer, "_emojiCompletion", "windows-emoji");
            Check(Press(input, Key.Down).Handled, "Arrow selects an emoji result");
            Check(Press(input, Key.Enter).Handled && sent == 0 && !input.Text.Contains(":sm"), "Enter selects before sending");
            input.Undo(); Pump();
            Check(input.Text == "hi :sm", "Completion is one reversible editor change");
            Check(!Press(input, Key.Down).Handled, "Undo does not reopen completion");
            input.Clear(); input.AppendText(":fire:"); input.CaretIndex = input.Text.Length; Pump();
            Check(input.Text == "🔥", "Closing an exact shortcode inserts the emoji");
            input.Undo(); Pump();
            Check(input.Text == ":fire:", "Undo does not reapply the shortcode");
            composer.RestoreDraft(":sm"); Pump();
            Check(!Press(input, Key.Down).Handled, "Restoring a draft never opens completion");
            input.AppendText("i"); input.CaretIndex = input.Text.Length; Pump();
            Check(Press(input, Key.Escape).Handled && input.Text == ":smi", "Escape preserves the draft");
        }
        finally { window.Content = previous; }
        Console.WriteLine("PASS: emoji token boundaries, keyboard selection, undo and restored drafts");
    }

    private static KeyEventArgs Press(TextBox input, Key key)
    {
        var args = new KeyEventArgs(Keyboard.PrimaryDevice, PresentationSource.FromVisual(input), 0, key) { RoutedEvent = Keyboard.PreviewKeyDownEvent };
        input.RaiseEvent(args); Pump(); return args;
    }

    private static void Pump()
    {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }

    private static void Check(bool passed, string message) { if (!passed) throw new Exception(message); }
}
