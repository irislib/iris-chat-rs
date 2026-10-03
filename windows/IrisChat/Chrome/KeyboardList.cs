using System;
using System.Linq;
using System.Collections.Generic;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;

namespace IrisChat.Chrome;

// Keep the sidebar a single Tab stop; arrows browse without opening a chat.
internal static class KeyboardList
{
    public static void Install(ItemsControl list)
    {
        KeyboardNavigation.SetTabNavigation(list, KeyboardNavigationMode.Once);
        list.PreviewKeyDown += (_, e) =>
        {
            if (Keyboard.Modifiers != ModifierKeys.None) return;
            var rows = Rows(list);
            var index = Array.FindIndex(rows, row => row.IsKeyboardFocusWithin);
            if (index < 0) return;
            var next = e.Key switch
            {
                Key.Up => Math.Max(0, index - 1),
                Key.Down => Math.Min(rows.Length - 1, index + 1),
                Key.Home => 0,
                Key.End => rows.Length - 1,
                _ => -1,
            };
            if (next < 0) return;
            rows[next].Focus();
            rows[next].BringIntoView();
            e.Handled = true;
        };
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
        public FocusRestore(ItemsControl list)
        {
            _list = list;
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
            }));
        }
    }
}
