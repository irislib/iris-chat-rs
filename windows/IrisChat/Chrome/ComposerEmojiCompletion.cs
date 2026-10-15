using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

internal sealed class ComposerEmojiCompletion
{
    private static readonly Lazy<EmojiCompletionEntry[]> Catalog = new(() => Native.IrisEmojiSearchAliases()
        .Select(value => (value.emoji, names: value.keywords.Split(' ', StringSplitOptions.RemoveEmptyEntries)))
        .Where(value => value.names.Length > 0)
        .Select(value => new EmojiCompletionEntry(value.emoji, value.names[0], value.names)).ToArray());
    private readonly TextBox _input;
    private readonly Func<bool> _available;
    private readonly Popup _popup;
    private readonly StackPanel _rows = new();
    private IReadOnlyList<EmojiCompletionEntry> _results = [];
    private EmojiCompletionToken? _token;
    private string _text = "";
    private int _caret;
    private int _selected;
    private int _generation;
    private bool _applying;
    private bool _composing;
    private bool _pending;

    public ComposerEmojiCompletion(TextBox input, Func<bool> available)
    {
        _input = input;
        _available = available;
        var border = new Border { Child = _rows, CornerRadius = new CornerRadius(10), Padding = new Thickness(4), MinWidth = 260 };
        border.SetResourceReference(Border.BackgroundProperty, "Panel");
        _popup = new Popup { Child = border, PlacementTarget = input, Placement = PlacementMode.Top, StaysOpen = true, AllowsTransparency = true };
        AutomationProperties.SetAutomationId(border, "chatEmojiSuggestions");
        input.TextChanged += (_, e) =>
        {
            if (_applying) return;
            Dismiss();
            if (!_available() || e.UndoAction is UndoAction.Undo or UndoAction.Redo) return;
            ScheduleUpdate();
        };
        input.SelectionChanged += (_, _) =>
        {
            if (!_applying && !_pending && input.Text == _text && (input.CaretIndex != _caret || input.SelectionLength != 0)) Dismiss();
        };
        input.LostKeyboardFocus += (_, _) => Dismiss();
        input.Unloaded += (_, _) => Dismiss();
        input.PreviewMouseDown += (_, _) => Dismiss();
        TextCompositionManager.AddPreviewTextInputStartHandler(input, (_, _) => { _composing = true; Dismiss(); });
        TextCompositionManager.AddPreviewTextInputUpdateHandler(input, (_, _) => { _composing = true; Dismiss(); });
        TextCompositionManager.AddPreviewTextInputHandler(input, (_, _) => { _composing = false; ScheduleUpdate(); });
    }

    public void Dismiss()
    {
        _generation++;
        _pending = false;
        _results = [];
        _token = null;
        _popup.IsOpen = false;
    }

    private void ScheduleUpdate()
    {
        var generation = ++_generation;
        _pending = true;
        _input.Dispatcher.BeginInvoke(DispatcherPriority.Background, new Action(() =>
        {
            if (generation == _generation) Update();
        }));
    }

    private void Update()
    {
        _pending = false;
        if (!_available() || _composing || !_input.IsKeyboardFocused) { Dismiss(); return; }
        _text = _input.Text;
        _caret = _input.CaretIndex;
        _token = EmojiCompletionToken.Find(_text, _caret, _input.SelectionLength);
        if (_token == null) { Dismiss(); return; }
        if (_token.Closed)
        {
            var exact = Catalog.Value.FirstOrDefault(entry => entry.MatchesExactly(_token.Query));
            if (exact != null) Insert(exact, false);
            else Dismiss();
            return;
        }
        _results = EmojiCompletionEntry.Search(_token.Query, Catalog.Value);
        _selected = 0;
        Render();
    }

    public bool HandleKey(KeyEventArgs e)
    {
        if (_composing || Keyboard.Modifiers != ModifierKeys.None) return false;
        if (_pending) Update();
        if (_results.Count == 0) return false;
        switch (e.Key)
        {
            case Key.Up: _selected = (_selected + _results.Count - 1) % _results.Count; Render(); return true;
            case Key.Down: _selected = (_selected + 1) % _results.Count; Render(); return true;
            case Key.Enter: case Key.Tab: Insert(_results[_selected], true); return true;
            case Key.Escape: Dismiss(); return true;
            case Key.Left: case Key.Right: case Key.Home: case Key.End: Dismiss(); return false;
            default: return false;
        }
    }

    private void Insert(EmojiCompletionEntry entry, bool trailingSpace)
    {
        var token = _token;
        if (token == null || _composing || _input.Text != _text || _input.CaretIndex != _caret || _input.SelectionLength != 0) { Dismiss(); return; }
        var tail = token.Start + token.Length;
        var replacement = entry.Emoji + (trailingSpace && (tail == _text.Length || !char.IsWhiteSpace(_text[tail])) ? " " : "");
        _applying = true;
        try
        {
            _input.BeginChange();
            try
            {
                _input.Select(token.Start, token.Length);
                _input.SelectedText = replacement;
                _input.Select(token.Start + replacement.Length, 0);
            }
            finally { _input.EndChange(); }
        }
        finally { _applying = false; Dismiss(); }
    }

    private void Render()
    {
        _rows.Children.Clear();
        for (int i = 0; i < _results.Count; i++)
        {
            var entry = _results[i];
            var row = new Button
            {
                Content = $"{entry.Emoji}   :{entry.Label}:", Focusable = false, Cursor = Cursors.Hand,
                HorizontalContentAlignment = HorizontalAlignment.Left, Padding = new Thickness(10, 7, 10, 7),
                BorderThickness = new Thickness(0), Background = i == _selected ? (Brush)_input.FindResource("PanelAlt") : Brushes.Transparent,
            };
            row.SetResourceReference(Control.ForegroundProperty, "TextPrimary");
            AutomationProperties.SetAutomationId(row, "chatEmojiCompletion-" + entry.Label);
            row.Click += (_, _) => Insert(entry, true);
            _rows.Children.Add(row);
        }
        _popup.IsOpen = _results.Count > 0;
    }
}
