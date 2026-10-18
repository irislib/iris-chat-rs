using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Automation;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Threading;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

internal sealed class ComposerMentionCompletion
{
    private readonly TextBox _input;
    private readonly Action<int, string, string> _inserted;
    private readonly Func<bool> _available;
    private int _generation;
    private bool _pending;
    private readonly Popup _popup;
    private readonly StackPanel _items = new();
    private readonly ScrollViewer _scroll;
    private IReadOnlyList<ChatParticipantSnapshot> _people = Array.Empty<ChatParticipantSnapshot>();
    private List<ChatParticipantSnapshot> _results = new();
    private string? _snapshot, _dismissed;
    private (int start, int end, string query)? _token;
    private int _selection;
    private bool _composing, _inserting;
    internal ComposerMentionCompletion(TextBox input, Func<bool> available, Action<int, string, string> inserted)
    {
        _input = input; _available = available; _inserted = inserted;
        _scroll = new ScrollViewer { Content = _items, MaxHeight = 240,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        var border = new Border { CornerRadius = new CornerRadius(16), Padding = new Thickness(4), Child = _scroll };
        border.SetResourceReference(Border.BackgroundProperty, "Panel");
        _popup = new Popup { PlacementTarget = input, Placement = PlacementMode.Top, StaysOpen = true, AllowsTransparency = true, Child = border };
        AutomationProperties.SetAutomationId(border, "chatMentionSuggestions");
        input.TextChanged += (_, e) => {
            if (_inserting) return;
            Dismiss();
            if (!_available() || e.UndoAction is UndoAction.Undo or UndoAction.Redo) return;
            _dismissed = null;
            ScheduleUpdate();
        };
        input.SelectionChanged += (_, _) => { if (!_pending && !_inserting && _snapshot == input.Text && _token is {} token && (input.CaretIndex != token.start + 1 + token.query.Length || input.SelectionLength != 0)) Dismiss(); };
        input.LostKeyboardFocus += (_, _) => Dismiss();
        input.Unloaded += (_, _) => Dismiss();
        input.PreviewMouseDown += (_, _) => Dismiss();
        TextCompositionManager.AddPreviewTextInputStartHandler(input, (_, _) => { _composing = true; Dismiss(); });
        TextCompositionManager.AddPreviewTextInputUpdateHandler(input, (_, _) => { _composing = true; Dismiss(); });
        TextCompositionManager.AddPreviewTextInputHandler(input, (_, _) => { _composing = false; ScheduleUpdate(); });
    }
    internal void SetPeople(IReadOnlyList<ChatParticipantSnapshot> people) { _people = people; }
    internal void Dismiss() { _generation++; _pending = false; _dismissed = _input.Text; _results.Clear(); _popup.IsOpen = false; }
    internal static (int start, int end, string query)? Find(string text, int caret, int selectionLength)
    {
        if (selectionLength != 0 || caret < 1 || caret > text.Length) return null;
        var start = text.LastIndexOf('@', caret - 1);
        if (start < 0 || (start > 0 && !char.IsWhiteSpace(text[start - 1]))) return null;
        var query = text[(start + 1)..caret];
        if (query.Length > 64 || query.Any(c => !char.IsLetterOrDigit(c) && c != ' ' && c != '_' && c != '-')) return null;
        var end = caret;
        while (end < text.Length && (char.IsLetterOrDigit(text[end]) || text[end] == '_' || text[end] == '-'))
            if (++end - start > 65) return null;
        return (start, end, query);
    }
    private void ScheduleUpdate()
    {
        var generation = ++_generation; _pending = true;
        _input.Dispatcher.BeginInvoke(DispatcherPriority.Background, new Action(() => { if (generation == _generation) Update(); }));
    }
    private void Update()
    {
        _pending = false;
        if (!_available() || _inserting || _composing || !_input.IsKeyboardFocusWithin || _input.Text == _dismissed) return;
        var token = Find(_input.Text, _input.CaretIndex, _input.SelectionLength);
        if (token == null) { _results.Clear(); _popup.IsOpen = false; return; }
        _snapshot = _input.Text; _token = token; _selection = 0;
        _results = _people.Where(p => p.displayName.Contains(token.Value.query, StringComparison.CurrentCultureIgnoreCase)).ToList();
        Draw();
    }
    private void Draw()
    {
        _items.Children.Clear();
        for (var i = 0; i < _results.Count; i++)
        {
            var index = i; var person = _results[i];
            var duplicate = _results.Count(p => p.displayName == person.displayName) > 1;
            var row = new StackPanel { Orientation = Orientation.Horizontal };
            row.Children.Add(new Avatar { OwnerPubkeyHex = person.ownerPubkeyHex, Label = person.displayName,
                Size = 32, ShowsNearbyBadge = false, Margin = new Thickness(0, 0, 10, 0) });
            row.Children.Add(new TextBlock { Text = person.displayName + (duplicate ? "  · " + person.ownerPubkeyHex[..8] : ""),
                VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis });
            var button = new Button { Content = row,
                Focusable = false, Cursor = Cursors.Hand, HorizontalContentAlignment = HorizontalAlignment.Left,
                MinWidth = 220, Padding = new Thickness(10, 7, 10, 7) };
            AutomationProperties.SetAutomationId(button, "chatMentionCompletion-" + person.ownerPubkeyHex);
            button.SetResourceReference(Control.StyleProperty, "GhostButton");
            if (index == _selection) button.SetResourceReference(Control.BackgroundProperty, "PanelAlt");
            button.Click += (_, _) => Insert(index);
            _items.Children.Add(button);
        }
        _popup.IsOpen = _results.Count > 0;
        if (_results.Count > 0) {
            _items.UpdateLayout();
            ((FrameworkElement)_items.Children[_selection]).BringIntoView();
        }
    }
    internal bool Handle(KeyEventArgs e)
    {
        if (_composing || Keyboard.Modifiers != ModifierKeys.None) return false;
        if (_pending) Update();
        if (!_popup.IsOpen || _results.Count == 0) return false;
        if (e.Key == Key.Escape) Dismiss();
        else if (e.Key == Key.Up || e.Key == Key.Down) { _selection = (_selection + (e.Key == Key.Up ? _results.Count - 1 : 1)) % _results.Count; Draw(); }
        else if (e.Key == Key.Enter || e.Key == Key.Tab) Insert(_selection);
        else if (e.Key == Key.Left || e.Key == Key.Right || e.Key == Key.Home || e.Key == Key.End) { Dismiss(); return false; }
        else return false;
        e.Handled = true; return true;
    }
    private void Insert(int index)
    {
        if (_composing || _snapshot != _input.Text || _token is not {} token || Find(_input.Text, _input.CaretIndex, _input.SelectionLength) != token) { Dismiss(); return; }
        var person = _results[index]; var label = "@" + person.displayName;
        var wire = Native.FormatMessageMention(person.ownerPubkeyHex, person.displayName);
        if (wire.Length == 0) return;
        var space = token.end == _input.Text.Length || !char.IsWhiteSpace(_input.Text[token.end]) ? " " : "";
        _inserting = true;
        try
        {
            _input.LockCurrentUndoUnit();
            _input.BeginChange();
            try
            {
                _input.Select(token.start, token.end - token.start);
                _input.SelectedText = label + space;
                _input.CaretIndex = token.start + label.Length + space.Length;
            }
            finally { _input.EndChange(); }
            _inserted(token.start, label, wire);
            _input.LockCurrentUndoUnit();
        }
        finally { _inserting = false; Dismiss(); }
    }
}
