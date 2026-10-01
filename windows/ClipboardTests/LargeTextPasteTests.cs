using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using IrisChat.Chrome;

internal static class LargeTextPasteTests
{
    internal static void Verify(ComposerBar composer, Action pump, string output)
    {
        var input = (TextBox)composer.FindName("Input");
        const string original = "Before [replace me] after";
        const int selectionStart = 7, selectionLength = 12;
        var text = string.Concat(Enumerable.Repeat("Unicode 世界🙂\r\nnext\n\ttab\r", 2048));
        var expected = original[..selectionStart] + text.Replace('\t', ' ') + original[(selectionStart + selectionLength)..];
        var caret = selectionStart + text.Length;
        composer.Clear();
        input.Text = original;
        input.Select(selectionStart, selectionLength);
        input.IsUndoEnabled = false;
        input.IsUndoEnabled = true;
        input.Focus();
        pump();
        Clipboard.SetText(text);
        var changes = 0;
        TextChangedEventHandler changed = (_, _) => changes++;
        input.TextChanged += changed;
        var elapsed = Stopwatch.StartNew();
        ApplicationCommands.Paste.Execute(null, input);
        var pasteExecuteMs = elapsed.Elapsed.TotalMilliseconds;
        // Edit before any dispatcher drain: deferring expensive validation to
        // the next edit must not make the paste optimization appear to pass.
        elapsed.Restart();
        input.SelectedText = "!";
        var immediateEditMs = elapsed.Elapsed.TotalMilliseconds;
        input.TextChanged -= changed;
        File.WriteAllText(Path.Combine(output, "windows-immediate-edit-timings.json"), JsonSerializer.Serialize(new
        {
            utf16_code_units = text.Length, utf8_bytes = Encoding.UTF8.GetByteCount(text),
            paste_execute_ms = pasteExecuteMs, immediate_edit_ms = immediateEditMs,
            text_change_events = changes, command_budget_ms = 500,
            boundary = "Native Paste Execute then SelectedText insertion before any dispatcher drain."
        }, new JsonSerializerOptions { WriteIndented = true }));
        Check(input.Text == expected.Insert(caret, "!") && changes == 2,
            "large paste and immediate edit each change the native editor once");
        Check(pasteExecuteMs < 500 && immediateEditMs < 500,
            "bulk text command and immediate edit must not synchronously format the entire document");
        input.Undo();
        Check(input.Text == expected && input.SelectionStart == caret && input.SelectionLength == 0,
            "one Undo removes only the subsequent edit and restores the pasted caret");
        input.Undo();
        Check(input.Text == original && input.SelectionStart == selectionStart && input.SelectionLength == selectionLength,
            "one Undo restores the entire replaced selection");
        input.Redo();
        Check(input.Text == expected && input.SelectionStart == caret && input.SelectionLength == 0,
            "one Redo restores Unicode, line endings, native tab filtering and final caret");
        pump();

        composer.Clear();
        input.Text = original;
        input.Select(selectionStart, selectionLength);
        var pasteEvents = 0;
        DataObjectPastingEventHandler cancel = (_, e) => { pasteEvents++; e.CancelCommand(); };
        DataObject.AddPastingHandler(input, cancel);
        ApplicationCommands.Paste.Execute(null, input);
        DataObject.RemovePastingHandler(input, cancel);
        Check(pasteEvents == 1 && input.Text == original && input.SelectionStart == selectionStart && input.SelectionLength == selectionLength,
            "Pasting cancellation preserves text and selection without running native paste again");

        DataObjectPastingEventHandler replace = (_, e) =>
        {
            pasteEvents++;
            e.DataObject = new DataObject(DataFormats.UnicodeText, "replacement🙂");
            e.FormatToApply = DataFormats.UnicodeText;
        };
        DataObject.AddPastingHandler(input, replace);
        ApplicationCommands.Paste.Execute(null, input);
        DataObject.RemovePastingHandler(input, replace);
        Check(pasteEvents == 2 && input.Text == "Before replacement🙂 after",
            "custom Pasting data replacement is respected once");

        composer.Clear();
        input.MaxLength = 8;
        Clipboard.SetText(new string('a', 40_000));
        ApplicationCommands.Paste.Execute(null, input);
        Check(input.Text == "aaaaaaaa", "constrained editors retain WPF's native paste filtering");
        input.MaxLength = 0;
        composer.Clear();
        Clipboard.SetText("small\t世界🙂\r\ntext");
        ApplicationCommands.Paste.Execute(null, input);
        Check(input.Text == "small 世界🙂\r\ntext", "ordinary native plaintext paste keeps its semantics");
        composer.Clear();
        pump();
        Console.WriteLine($"PASS: large text selection, Unicode/newlines/tabs, cancellation, replacement, undo/redo, native constraints; immediate edit {immediateEditMs:F1} ms");
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
