using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

internal static class ComposerViewport
{
    internal static Rect Bounds(TextBox input)
    {
        var host = input.Template.FindName("PART_ContentHost", input) as ScrollViewer
            ?? throw new InvalidOperationException("Native text scroll host is missing");
        var presenter = FindPresenter(host)
            ?? throw new InvalidOperationException("Native text viewport is missing");
        return presenter.TransformToAncestor(input).TransformBounds(new Rect(presenter.RenderSize));
    }

    internal static bool Contains(TextBox input, Rect caret)
    {
        var viewport = Bounds(input);
        return !caret.IsEmpty && caret.Top >= viewport.Top - 0.5 && caret.Bottom <= viewport.Bottom + 0.5;
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
