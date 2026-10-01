using System.Windows.Controls;

namespace IrisChat;

internal static class ComposerTextLayout
{
    internal const int BulkLength = 32 * 1024;

    internal static void Update(TextBox input, int length)
    {
        // Auto first measures without a scrollbar, then formats a large draft
        // again at its narrower width. Reserve that width before the first layout.
        var visibility = length >= BulkLength ? ScrollBarVisibility.Visible : ScrollBarVisibility.Auto;
        if (input.VerticalScrollBarVisibility != visibility)
            input.SetCurrentValue(ScrollViewer.VerticalScrollBarVisibilityProperty, visibility);
    }

    internal static void CompletePaste(TextBox input)
    {
        // Finish the tail once, within the paste operation. Background formatting
        // alone can leave the new caret outside the viewport after a bulk insert.
        var caret = input.GetRectFromCharacterIndex(input.CaretIndex);
        if (caret.IsEmpty || (caret.Top >= 0 && caret.Bottom <= input.ActualHeight)) return;
        var line = input.GetLineIndexFromCharacterIndex(input.CaretIndex);
        if (line < 0) return;
        input.ScrollToLine(line);
        input.UpdateLayout();
    }
}
