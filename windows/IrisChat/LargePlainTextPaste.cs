using System;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat;

internal static class LargePlainTextPaste
{
    // Keep everyday pastes on WPF's command. Bulk documents can make its
    // full-content ValidateLayout synchronously format thousands of lines.
    private const int MinimumLength = 32 * 1024;

    internal static bool TryPaste(TextBox input, IDataObject? data, Func<bool> isCurrent)
    {
        if (data == null || input.IsReadOnly || !input.IsEnabled || !input.AcceptsReturn ||
            input.MaxLength != 0 || input.CharacterCasing != CharacterCasing.Normal ||
            !data.GetDataPresent(DataFormats.UnicodeText) ||
            data.GetData(DataFormats.UnicodeText) is not string text || text.Length < MinimumLength)
            return false;

        // Canceling from inside WPF's own Pasting event still runs its forced
        // layout. Handle PreviewExecuted instead and preserve the public event.
        if (!isCurrent()) return true;
        var pasting = new DataObjectPastingEventArgs(data, false, DataFormats.UnicodeText);
        input.RaiseEvent(pasting);
        if (pasting.CommandCancelled || !isCurrent()) return true;
        if (pasting.FormatToApply != DataFormats.UnicodeText && pasting.FormatToApply != DataFormats.Text)
            return true;
        var replacement = ReferenceEquals(pasting.DataObject, data) && pasting.FormatToApply == DataFormats.UnicodeText
            ? text : pasting.DataObject.GetData(pasting.FormatToApply) as string;
        if (replacement == null || !isCurrent()) return true;
        // Match the native multiline TextBox plaintext filter. Other casing,
        // length limits, or single-line editors continue through WPF above.
        if (!input.AcceptsTab) replacement = replacement.Replace('\t', ' ');
        if (replacement.Length == 0) return true;

        var start = input.SelectionStart;
        input.BeginChange();
        try
        {
            input.SelectedText = replacement;
            input.Select(start + replacement.Length, 0);
        }
        finally { input.EndChange(); }
        return true;
    }
}
