using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;

namespace IrisChat.Chrome;

/// One selection across the timeline's native text controls. Message metadata,
/// attachments and action buttons are deliberately outside the selection.
internal sealed class MessageTextSelection
{
    private readonly ScrollViewer _scroll;
    private readonly ItemsControl _list;
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromMilliseconds(33) };
    private List<MessageSelectionTextBox> _selected = new();
    private (MessageSelectionTextBox View, TextPointer Pointer)? _anchor;
    private Point _down;
    private bool _pending, _dragging;
    private Window? _window;

    public MessageTextSelection(ScrollViewer scroll, ItemsControl list)
    {
        _scroll = scroll; _list = list;
        scroll.PreviewMouseLeftButtonDown += Begin;
        scroll.PreviewMouseMove += Move;
        scroll.PreviewMouseLeftButtonUp += End;
        scroll.LostMouseCapture += (_, _) => { _timer.Stop(); _pending = _dragging = false; };
        scroll.PreviewKeyDown += (_, e) =>
        {
            if (e.Key == Key.Escape && SelectedText.Length > 0) { Clear(); e.Handled = true; }
            else if (e.Key is Key.Left or Key.Right or Key.Up or Key.Down or Key.Home or Key.End
                || e.Key == Key.A && Keyboard.Modifiers.HasFlag(ModifierKeys.Control)) Clear();
        };
        scroll.AddHandler(CommandManager.PreviewExecutedEvent, new ExecutedRoutedEventHandler((_, e) =>
        {
            if (e.Command == ApplicationCommands.Copy && Copy()) e.Handled = true;
        }));
        scroll.AddHandler(CommandManager.PreviewCanExecuteEvent, new CanExecuteRoutedEventHandler((_, e) =>
        {
            if (e.Command == ApplicationCommands.Copy && SelectedText.Length > 0)
            { e.CanExecute = true; e.Handled = true; }
        }));
        scroll.Loaded += (_, _) =>
        {
            _window = Window.GetWindow(scroll);
            if (_window != null) _window.PreviewMouseDown += WindowMouseDown;
        };
        scroll.Unloaded += (_, _) =>
        {
            if (_window != null) _window.PreviewMouseDown -= WindowMouseDown;
            _window = null; Clear();
        };
        _timer.Tick += (_, _) =>
        {
            if (!_dragging || Mouse.LeftButton != MouseButtonState.Pressed) { _timer.Stop(); return; }
            var p = Mouse.GetPosition(scroll);
            var delta = p.Y < 24 ? -12 : p.Y > scroll.ActualHeight - 24 ? 12 : 0;
            if (delta != 0) scroll.ScrollToVerticalOffset(scroll.VerticalOffset + delta);
            Extend(p);
        };
    }

    private void WindowMouseDown(object sender, MouseButtonEventArgs e)
    {
        if (e.ChangedButton == MouseButton.Left && !IsWithin(e.OriginalSource as DependencyObject, _scroll)) Clear();
    }

    private List<MessageSelectionTextBox> Bodies() => _list.Items.OfType<FrameworkElement>()
        .Select(row => row.FindName("BodyText")).OfType<MessageSelectionTextBox>()
        .Where(view => view.IsVisible && view.ActualHeight > 0).ToList();

    private void Begin(object sender, MouseButtonEventArgs e)
    {
        var view = Ancestor<MessageSelectionTextBox>(e.OriginalSource as DependencyObject);
        if (e.ClickCount != 1 || view == null) { Clear(); return; }
        var pointer = view.GetPositionFromPoint(e.GetPosition(view), true);
        if (pointer == null) return;
        var extend = Keyboard.Modifiers.HasFlag(ModifierKeys.Shift) && _anchor != null;
        if (!extend) { Clear(); _anchor = (view, pointer); }
        _down = e.GetPosition(_scroll);
        _pending = true;
        if (extend) { StartDrag(); Extend(_down); e.Handled = true; }
        // Ordinary clicks, links, double/triple clicks stay native until a drag.
    }

    private void Move(object sender, MouseEventArgs e)
    {
        if (!_pending || Mouse.LeftButton != MouseButtonState.Pressed) return;
        var p = e.GetPosition(_scroll);
        if (!_dragging && (Math.Abs(p.X - _down.X) >= SystemParameters.MinimumHorizontalDragDistance
            || Math.Abs(p.Y - _down.Y) >= SystemParameters.MinimumVerticalDragDistance)) StartDrag();
        if (!_dragging) return;
        Extend(p); e.Handled = true;
    }

    private void StartDrag()
    {
        // Capturing the scroll host also suppresses link activation on release.
        _scroll.CaptureMouse();
        _pending = _dragging = true;
        _anchor?.View.Focus();
        _timer.Start();
    }

    private void End(object sender, MouseButtonEventArgs e)
    {
        var dragged = _dragging;
        if (dragged) Extend(e.GetPosition(_scroll));
        _timer.Stop(); _pending = _dragging = false;
        if (_scroll.IsMouseCaptured) _scroll.ReleaseMouseCapture();
        if (dragged) e.Handled = true;
    }

    internal void Extend(Point point)
    {
        if (_anchor is not { } anchor) return;
        var bodies = Bodies();
        var a = bodies.IndexOf(anchor.View);
        if (a < 0) { Clear(); return; }
        var target = bodies.MinBy(view =>
        {
            var y = view.TranslatePoint(new Point(), _scroll).Y;
            return Math.Max(0, Math.Max(y - point.Y, point.Y - y - view.ActualHeight));
        });
        if (target == null) return;
        var local = _scroll.TranslatePoint(point, target);
        var end = local.Y < 0 ? target.Document.ContentStart : local.Y > target.ActualHeight
            ? target.Document.ContentEnd : target.GetPositionFromPoint(local, true);
        if (end == null) return;
        Select(bodies, anchor.View, anchor.Pointer, target, end);
    }

    internal void Select(List<MessageSelectionTextBox> bodies, MessageSelectionTextBox startView,
        TextPointer start, MessageSelectionTextBox endView, TextPointer end)
    {
        var a = bodies.IndexOf(startView); var b = bodies.IndexOf(endView);
        if (a < 0 || b < 0) { Clear(); return; }
        _anchor = (startView, start);
        var forward = a < b || a == b && start.CompareTo(end) <= 0;
        var selected = bodies.Skip(Math.Min(a, b)).Take(Math.Abs(a - b) + 1).ToList();
        foreach (var old in _selected.Where(view => !selected.Contains(view))) old.ClearMessageSelection();
        _selected = selected;
        foreach (var view in _selected)
        {
            view.SelectionOwner = this;
            var lower = view == (forward ? startView : endView) ? forward ? start : end : view.Document.ContentStart;
            var upper = view == (forward ? endView : startView) ? forward ? end : start : view.Document.ContentEnd;
            view.SelectMessageRange(lower, upper);
        }
    }

    internal string SelectedText => string.Join("\n", _selected.Select(view =>
    {
        var text = view.Selection.Text;
        // WPF appends a paragraph terminator to a complete FlowDocument.
        if (view.Selection.End.CompareTo(view.Document.ContentEnd) == 0 && text.EndsWith("\r\n")) text = text[..^2];
        return text;
    }).Where(text => text.Length > 0));

    internal bool Copy()
    {
        var text = SelectedText;
        if (text.Length == 0) return false;
        try { Clipboard.SetText(text); } catch (System.Runtime.InteropServices.ExternalException) { }
        return true;
    }

    internal void Reconcile()
    {
        var bodies = Bodies();
        if (_selected.Any(view => !bodies.Contains(view))) Clear();
    }

    internal void Clear()
    {
        _timer.Stop(); _pending = _dragging = false; _anchor = null;
        if (_scroll.IsMouseCaptured) _scroll.ReleaseMouseCapture();
        foreach (var view in _selected)
        { view.ClearMessageSelection(); view.SelectionOwner = null; }
        _selected.Clear();
    }

    private static DependencyObject? Parent(DependencyObject item) => item is Visual
        ? VisualTreeHelper.GetParent(item) : LogicalTreeHelper.GetParent(item);
    private static T? Ancestor<T>(DependencyObject? item) where T : DependencyObject
    {
        while (item != null) { if (item is T found) return found; item = Parent(item); }
        return null;
    }
    private static bool IsWithin(DependencyObject? item, DependencyObject root)
    {
        while (item != null) { if (ReferenceEquals(item, root)) return true; item = Parent(item); }
        return false;
    }
}

public sealed class MessageSelectionTextBox : RichTextBox
{
    internal MessageTextSelection? SelectionOwner { get; set; }
    private RangeHighlight? _highlight;
    private double _nativeSelectionOpacity;
    private string? _bodySource;

    internal void SetBodyDocument(string source, FlowDocument document)
    {
        // Delivery/footer updates rebind a bubble. Keep its native document and
        // selection when both the source and resolved mention labels are unchanged.
        if (_bodySource == source && new TextRange(Document.ContentStart, Document.ContentEnd).Text
            == new TextRange(document.ContentStart, document.ContentEnd).Text) return;
        _bodySource = source;
        Document = document;
    }

    internal void SelectMessageRange(TextPointer start, TextPointer end)
    {
        var changed = Selection.Start.CompareTo(start) != 0 || Selection.End.CompareTo(end) != 0;
        if (changed) Selection.Select(start, end);
        if (_highlight == null && AdornerLayer.GetAdornerLayer(this) is { } layer)
        {
            // WPF does not create a native selection adorner for a text control
            // that has never had keyboard focus. Draw every range without moving
            // focus (which would scroll rows or announce each bubble).
            _nativeSelectionOpacity = SelectionOpacity;
            SelectionOpacity = 0;
            _highlight = new RangeHighlight(this);
            layer.Add(_highlight);
            changed = true;
        }
        if (changed) _highlight?.Refresh();
    }

    internal void ClearMessageSelection()
    {
        if (_highlight != null)
        {
            AdornerLayer.GetAdornerLayer(this)?.Remove(_highlight);
            _highlight = null;
            SelectionOpacity = _nativeSelectionOpacity;
        }
        Selection.Select(Document.ContentStart, Document.ContentStart);
        SelectionOwner = null;
    }

    private sealed class RangeHighlight : Adorner
    {
        private readonly MessageSelectionTextBox _view;
        private Geometry? _geometry;
        public RangeHighlight(MessageSelectionTextBox view) : base(view)
        { _view = view; IsHitTestVisible = false; }
        public void Refresh() { _geometry = null; InvalidateVisual(); }
        protected override void OnRender(DrawingContext drawing)
        {
            if (_geometry == null)
            {
                var geometry = new GeometryGroup { FillRule = FillRule.Nonzero };
                var position = _view.Selection.Start;
                while (position.CompareTo(_view.Selection.End) < 0)
                {
                    var next = position.GetNextInsertionPosition(LogicalDirection.Forward);
                    if (next == null) break;
                    if (next.CompareTo(_view.Selection.End) > 0) next = _view.Selection.End;
                    var leading = position.GetCharacterRect(LogicalDirection.Forward);
                    var trailing = next.GetCharacterRect(LogicalDirection.Backward);
                    if (!leading.IsEmpty && !trailing.IsEmpty && Math.Abs(leading.Y - trailing.Y) < 1)
                    {
                        // Native insertion positions respect composed glyphs and
                        // bidirectional runs; a line can contain disjoint ranges.
                        var rect = new Rect(Math.Min(leading.X, trailing.X), leading.Y,
                            Math.Max(1, Math.Abs(trailing.X - leading.X)), Math.Max(leading.Height, trailing.Height));
                        rect.Intersect(new Rect(_view.RenderSize));
                        if (!rect.IsEmpty) geometry.Children.Add(new RectangleGeometry(rect));
                    }
                    position = next;
                }
                geometry.Freeze(); _geometry = geometry;
            }
            var brush = SystemColors.HighlightBrush.CloneCurrentValue();
            brush.Opacity = _view._nativeSelectionOpacity;
            drawing.DrawGeometry(brush, null, _geometry);
        }
    }

    public MessageSelectionTextBox()
    {
        IsInactiveSelectionHighlightEnabled = true;
        SizeChanged += (_, _) => _highlight?.Refresh();
        TextChanged += (_, _) => SelectionOwner?.Clear();
        ContextMenuOpening += (_, e) =>
        {
            if (SelectionOwner?.SelectedText.Length > 0)
            {
                var menu = new ContextMenu();
                var copy = new MenuItem { Header = "Copy" };
                copy.Click += (_, _) => SelectionOwner?.Copy();
                menu.Items.Add(copy); menu.PlacementTarget = this; menu.IsOpen = true;
                e.Handled = true;
            }
        };
    }
}
