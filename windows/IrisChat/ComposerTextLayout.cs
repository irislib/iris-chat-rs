using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace IrisChat;

internal static class ComposerTextLayout
{
    internal const int BulkLength = 32 * 1024;

    internal static bool Update(TextBox input, int length)
    {
        // Auto first measures without a scrollbar, then formats a large draft
        // again at its narrower width. Reserve that width before the first layout.
        var visibility = length >= BulkLength ? ScrollBarVisibility.Visible : ScrollBarVisibility.Auto;
        if (input.VerticalScrollBarVisibility == visibility) return false;
        input.SetCurrentValue(ScrollViewer.VerticalScrollBarVisibilityProperty, visibility);
        return true;
    }

    internal static void CompletePaste(TextBox input, PastePhaseDiagnostics? timing = null)
    {
        var caret = input.GetRectFromCharacterIndex(input.CaretIndex);
        timing?.Mark("caret_geometry");
        if (caret.IsEmpty) return;
        // The content presenter clips to RenderSize. ActualHeight also includes
        // border/padding, so it cannot tell whether the whole caret is visible.
        var host = input.Template.FindName("PART_ContentHost", input) as ScrollViewer;
        var presenter = host == null ? null : FindPresenter(host);
        var viewport = presenter?.TransformToAncestor(input).TransformBounds(new Rect(presenter.RenderSize));
        timing?.Mark("content_viewport_geometry");
        if (viewport is {} bounds && caret.Top >= bounds.Top && caret.Bottom <= bounds.Bottom) return;

        var line = input.GetLineIndexFromCharacterIndex(input.CaretIndex);
        timing?.Mark("caret_line_lookup");
        if (line < 0) return;
        // ScrollToLine validates the document end again; only pay for that work
        // when native Paste has actually left the caret outside its viewport.
        input.ScrollToLine(line);
        timing?.Mark("conditional_native_scroll");
        input.UpdateLayout();
        timing?.Mark("conditional_scroll_layout");
    }

    private static ScrollContentPresenter? FindPresenter(DependencyObject parent)
    {
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
        {
            var child = VisualTreeHelper.GetChild(parent, i);
            if (child is ScrollContentPresenter presenter) return presenter;
            if (FindPresenter(child) is {} nested) return nested;
        }
        return null;
    }
}
