using System;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat;

internal static class LargePlainTextPaste
{
    internal static bool TryPaste(TextBox input, IDataObject? data, Func<bool> isCurrent)
    {
        if (data == null || input.IsReadOnly || !input.IsEnabled || !input.AcceptsReturn ||
            !data.GetDataPresent(DataFormats.UnicodeText) ||
            data.GetData(DataFormats.UnicodeText) is not string text || text.Length < ComposerTextLayout.BulkLength)
            return false;
        if (!isCurrent()) return true;

        var timing = PastePhaseDiagnostics.Begin();
        DataObjectPastingEventHandler guard = (_, e) =>
        {
            if (!isCurrent()) { e.CancelCommand(); return; }
            if (e.CommandCancelled) return;
            // Native Paste flushes pending typing before this event. If that
            // changed the draft policy, settle the old text at the final width
            // again before WPF inserts clipboard content.
            if (ComposerTextLayout.Update(input, ComposerTextLayout.BulkLength)) input.UpdateLayout();
            // Snapshot the native plaintext choice while checking each provider
            // read. WPF's Unicode-null fallback reads SourceDataObject directly,
            // so wrapping only e.DataObject would leave that fallback unguarded.
            var format = e.FormatToApply;
            if (format != DataFormats.UnicodeText && format != DataFormats.Text) return;
            var value = ReadPasteData(e.DataObject, format, isCurrent);
            if (value == null && format == DataFormats.UnicodeText && e.DataObject.GetDataPresent(DataFormats.Text))
            {
                if (!isCurrent()) throw new StalePasteException();
                format = DataFormats.Text;
                value = ReadPasteData(e.SourceDataObject, format, isCurrent);
            }
            if (!isCurrent()) throw new StalePasteException();
            if (value == null) { e.CancelCommand(); return; }
            var replacement = value.ToString();
            if (!isCurrent()) throw new StalePasteException();
            e.DataObject = new DataObject(format, replacement);
            e.FormatToApply = format;
        };
        DataObject.AddPastingHandler(input, guard);
        try
        {
            ComposerTextLayout.Update(input, ComposerTextLayout.BulkLength);
            timing?.Mark("reserve_scrollbar_policy");
            input.UpdateLayout();
            timing?.Mark("existing_draft_layout");
            if (!isCurrent()) return true;
            // This public method invokes the native editor directly, without
            // routing another Paste command through PreviewExecuted. WPF owns
            // pending input, format/filter handling, selection and undo/redo.
            input.Paste();
            timing?.Mark("native_paste");
            if (isCurrent()) ComposerTextLayout.CompletePaste(input, timing);
        }
        catch (StalePasteException) { timing?.Mark("stale_native_paste"); }
        finally
        {
            DataObject.RemovePastingHandler(input, guard);
            // A Pasting handler may cancel or replace the large payload with a
            // short value, and native MaxLength may truncate it.
            ComposerTextLayout.Update(input, input.Text.Length);
            timing?.Mark("restore_actual_length_policy");
            timing?.Complete();
        }
        return true;
    }

    private sealed class StalePasteException : Exception { }

    // Match the native editor's unavailable-format handling; a destination
    // change throws a separate exception which its fallback catches do not eat.
    private static object? ReadPasteData(IDataObject source, string format, Func<bool> isCurrent)
    {
        if (!isCurrent()) throw new StalePasteException();
        try { return source.GetData(format, true); }
        catch (OutOfMemoryException) { return null; }
        catch (ExternalException) { return null; }
        finally { if (!isCurrent()) throw new StalePasteException(); }
    }
}
