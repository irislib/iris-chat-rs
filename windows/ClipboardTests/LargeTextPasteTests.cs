using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using IrisChat;
using IrisChat.Chrome;

internal static class LargeTextPasteTests
{
    internal static (double CompleteMs, double EditSettleMs) Verify(ComposerBar composer, Action pump, string output)
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
        using var probe = new PasteLayoutProbe();
        var elapsed = Stopwatch.StartNew();
        IReadOnlyDictionary<string, double>? nativePastePhases = null;
        PastePhaseDiagnostics.Observer = phases => nativePastePhases = phases;
        try { ApplicationCommands.Paste.Execute(null, input); }
        finally { PastePhaseDiagnostics.Observer = null; }
        var pasteExecuteMs = elapsed.Elapsed.TotalMilliseconds;
        // Edit before any dispatcher drain: deferring expensive validation to
        // the next edit must not make the paste optimization appear to pass.
        elapsed.Restart();
        input.SelectedText = "!";
        var immediateEditMs = elapsed.Elapsed.TotalMilliseconds;
        input.UpdateLayout(); pump();
        _ = input.GetRectFromCharacterIndex(input.CaretIndex);
        input.UpdateLayout(); pump();
        probe.WaitForFirstRender();
        var completedCaret = input.GetRectFromCharacterIndex(input.CaretIndex);
        elapsed.Stop();
        probe.Finish();
        var completeMs = probe.ElapsedMs;
        var editSettleMs = elapsed.Elapsed.TotalMilliseconds;
        input.TextChanged -= changed;
        File.WriteAllText(Path.Combine(output, "windows-immediate-edit-timings.json"), JsonSerializer.Serialize(new
        {
            utf16_code_units = text.Length, utf8_bytes = Encoding.UTF8.GetByteCount(text),
            paste_execute_ms = pasteExecuteMs, immediate_edit_ms = immediateEditMs,
            native_paste_phases_ms = nativePastePhases,
            complete_paste_and_edit_ms = completeMs, subsequent_edit_settle_ms = editSettleMs,
            first_render_ms = probe.FirstRenderMs, longest_dispatcher_gap_ms = probe.LongestDispatcherGapMs,
            text_change_events = changes, subsequent_edit_budget_ms = 500, complete_operation_budget_ms = 5_000,
            boundary = "Native Paste Execute then SelectedText insertion before any dispatcher drain; complete-operation and subsequent-edit times include forced layout, drain, refreshed current-caret geometry and first WPF Rendering callback (not display scanout)."
        }, new JsonSerializerOptions { WriteIndented = true }));
        Check(input.Text == expected.Insert(caret, "!") && changes == 2,
            "large paste and immediate edit each change the native editor once");
        Check(input.VerticalScrollBarVisibility == ScrollBarVisibility.Visible && ComposerViewport.Contains(input, completedCaret),
            "bulk draft reserves scrollbar width and keeps the complete edited caret visible");
        input.Undo();
        Check(input.Text == expected && input.SelectionStart == caret && input.SelectionLength == 0,
            "one Undo removes only the subsequent edit and restores the pasted caret");
        input.Undo();
        Check(input.Text == original && input.SelectionStart == selectionStart && input.SelectionLength == selectionLength,
            "one Undo restores the entire replaced selection");
        Check(input.VerticalScrollBarVisibility == ScrollBarVisibility.Auto,
            "Undo restores the normal small-draft appearance");
        input.Redo();
        Check(input.Text == expected && input.SelectionStart == caret && input.SelectionLength == 0,
            "one Redo restores Unicode, line endings, native tab filtering and final caret");
        Check(input.VerticalScrollBarVisibility == ScrollBarVisibility.Visible,
            "Redo restores bulk layout without changing the selection");
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
        Check(input.VerticalScrollBarVisibility == ScrollBarVisibility.Auto,
            "canceled bulk paste restores actual small-draft scrollbar policy");

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
        Check(input.VerticalScrollBarVisibility == ScrollBarVisibility.Auto,
            "short replacement restores actual small-draft scrollbar policy");

        composer.Clear();
        input.MaxLength = 8;
        Clipboard.SetText(new string('a', 40_000));
        ApplicationCommands.Paste.Execute(null, input);
        Check(input.Text == "aaaaaaaa", "constrained editors retain WPF's native paste filtering");
        Check(input.VerticalScrollBarVisibility == ScrollBarVisibility.Auto,
            "native length filtering restores the actual small-draft policy");
        input.MaxLength = 0;
        composer.Clear();
        input.CharacterCasing = CharacterCasing.Upper;
        ApplicationCommands.Paste.Execute(null, input);
        Check(input.Text == new string('A', 40_000), "large paste retains native character casing");
        input.CharacterCasing = CharacterCasing.Normal;
        composer.Clear();
        Clipboard.SetText("small\t世界🙂\r\ntext");
        ApplicationCommands.Paste.Execute(null, input);
        Check(input.Text == "small 世界🙂\r\ntext", "ordinary native plaintext paste keeps its semantics");
        Check(input.VerticalScrollBarVisibility == ScrollBarVisibility.Auto,
            "ordinary small drafts keep automatic scrollbars");
        NativePasteDestinationTests.Verify(composer, input, text, output);
        VerifyPartiallyClippedCaret(composer, input, text, pump);
        composer.Clear();
        pump();
        Console.WriteLine($"PASS: large text selection, Unicode/newlines/tabs, cancellation, replacement, undo/redo, native constraints, delayed destination guard and actual caret viewport; immediate edit {immediateEditMs:F1} ms");
        return (completeMs, editSettleMs);
    }

    private static void VerifyPartiallyClippedCaret(ComposerBar composer, TextBox input, string text, Action pump)
    {
        composer.Clear();
        input.Text = "prefix\n" + string.Concat(Enumerable.Repeat("tail\n", 256));
        input.CaretIndex = 7;
        pump();
        Clipboard.SetText(text);
        var prepared = false;
        var clippedInPadding = false;
        TextChangedEventHandler clipIntoPadding = (_, _) =>
        {
            if (prepared) return;
            prepared = true;
            var caret = input.GetRectFromCharacterIndex(input.CaretIndex);
            var viewport = ComposerViewport.Bounds(input);
            input.ScrollToVerticalOffset(input.VerticalOffset + caret.Bottom - viewport.Bottom - 4);
            input.UpdateLayout();
            caret = input.GetRectFromCharacterIndex(input.CaretIndex);
            clippedInPadding = caret.Bottom > viewport.Bottom + 1 && caret.Bottom <= input.ActualHeight;
        };
        input.TextChanged += clipIntoPadding;
        try { ApplicationCommands.Paste.Execute(null, input); }
        finally { input.TextChanged -= clipIntoPadding; }
        Check(prepared && clippedInPadding,
            "native fixture places the pasted caret partly inside bottom padding");
        Check(ComposerViewport.Contains(input, input.GetRectFromCharacterIndex(input.CaretIndex)),
            "bulk paste finishes with the complete caret in the actual content viewport");
        pump();
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
