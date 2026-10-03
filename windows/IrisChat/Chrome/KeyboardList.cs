using System;
using System.Linq;
using System.Collections.Generic;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using System.Windows.Media;

namespace IrisChat.Chrome;

// Tab visits native chat rows; arrows scroll without changing focus or chat.
internal static class KeyboardList
{
    public static void Install(ItemsControl list)
    {
        // ItemsControl inherits Control's focusable Tab stop. The container
        // groups navigation, but only its chat rows should receive focus.
        list.Focusable = false;
        KeyboardNavigation.SetIsTabStop(list, false);
        KeyboardNavigation.SetTabNavigation(list, KeyboardNavigationMode.Continue);
        list.PreviewKeyDown += (_, e) =>
        {
            if (Keyboard.Modifiers != ModifierKeys.None) return;
            var rows = Rows(list);
            var index = Array.FindIndex(rows, row => row.IsKeyboardFocusWithin);
            if (index < 0) return;
            var scroll = Scroll(list);
            if (scroll == null) return;
            switch (e.Key)
            {
                case Key.Up: scroll.ScrollToVerticalOffset(scroll.VerticalOffset - 40); break;
                case Key.Down: scroll.ScrollToVerticalOffset(scroll.VerticalOffset + 40); break;
                case Key.Home: scroll.ScrollToTop(); break;
                case Key.End: scroll.ScrollToBottom(); break;
                default: return;
            }
            e.Handled = true;
        };
    }

    public static bool Focus(ItemsControl list) =>
        (Rows(list).FirstOrDefault(row => row is ChatRow { IsActive: true }) ?? Rows(list).FirstOrDefault())?.Focus() ?? false;

    private static ScrollViewer? Scroll(DependencyObject widget)
    {
        for (var parent = VisualTreeHelper.GetParent(widget); parent != null; parent = VisualTreeHelper.GetParent(parent))
            if (parent is ScrollViewer scroll) return scroll;
        return null;
    }

    public static IDisposable PreserveFocus(ItemsControl list) => new FocusRestore(list);

    private static FrameworkElement[] Rows(ItemsControl list) => list.Items
        .OfType<FrameworkElement>().SelectMany(FocusableRows).ToArray();

    private static IEnumerable<FrameworkElement> FocusableRows(FrameworkElement element)
    {
        if (!element.IsEnabled || element.Visibility != Visibility.Visible) yield break;
        if (element.Focusable) { yield return element; yield break; }
        foreach (var child in LogicalTreeHelper.GetChildren(element).OfType<FrameworkElement>())
            foreach (var row in FocusableRows(child)) yield return row;
    }

    private sealed class FocusRestore : IDisposable
    {
        private readonly ItemsControl _list;
        private readonly FrameworkElement? _previous;
        private readonly int _index;
        private readonly double? _offset;
        public FocusRestore(ItemsControl list)
        {
            _list = list;
            _offset = Scroll(list)?.VerticalOffset;
            var rows = Rows(list);
            _index = Array.FindIndex(rows, row => row.IsKeyboardFocusWithin);
            _previous = _index < 0 ? null : rows[_index];
        }
        public void Dispose()
        {
            if (_previous == null) return;
            _list.Dispatcher.BeginInvoke(DispatcherPriority.Loaded, new Action(() =>
            {
                // An opened chat or a subsequent key press may already have moved focus.
                if (Keyboard.FocusedElement is { } focused && focused != _previous
                    && !_list.IsKeyboardFocusWithin) return;
                var rows = Rows(_list);
                var row = rows.FirstOrDefault(row => _previous.Uid.Length > 0 && row.Uid == _previous.Uid)
                    ?? rows.ElementAtOrDefault(Math.Min(_index, rows.Length - 1));
                row?.Focus();
                if (_offset is { } offset) Scroll(_list)?.ScrollToVerticalOffset(offset);
            }));
        }
    }
}
