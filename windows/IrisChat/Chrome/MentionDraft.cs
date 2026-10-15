using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows.Controls;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

/// UTF-16 ranges track selected identities independently of their displayed name.
internal sealed class MentionDraft
{
    private sealed record Span(int Start, int Length, string Label, string Token);
    private List<Span> _spans = new();
    private string _text = "";
    private sealed record Snapshot(string Text, Span[] Spans);
    private sealed record UndoUnit(Snapshot Before, Snapshot After);
    private readonly List<UndoUnit> _history = new();
    private int _position;

    private Snapshot Capture() => new(_text, _spans.ToArray());
    private void ClearHistory() { _history.Clear(); _position = 0; }

    internal string Restore(string wire)
    {
        _spans.Clear();
        ClearHistory();
        var text = wire;
        var shift = 0;
        foreach (var mention in Native.MessageMentions(wire))
        {
            var start = (int)mention.start + shift;
            var length = (int)mention.length;
            var label = MentionText.Label(mention);
            var token = text.Substring(start, length);
            text = text.Remove(start, length).Insert(start, label);
            _spans.Add(new(start, label.Length, label, token));
            shift += label.Length - length;
        }
        return _text = text;
    }

    internal void Changed(string text, IEnumerable<TextChange> changes, UndoAction action, int undoLimit)
    {
        if (action == UndoAction.Undo && _position > 0 && RestoreSnapshot(_history[_position - 1].Before, text))
        {
            _position--;
            return;
        }
        if (action == UndoAction.Redo && _position < _history.Count && RestoreSnapshot(_history[_position].After, text))
        {
            _position++;
            return;
        }
        var before = Capture();
        ApplyChanges(text, changes);
        var after = Capture();
        if (action is not (UndoAction.Create or UndoAction.Merge) || undoLimit == 0)
        {
            // An untracked native history reset must never recover an identity
            // by finding another occurrence of the same displayed name.
            ClearHistory();
            return;
        }
        if (_position < _history.Count) _history.RemoveRange(_position, _history.Count - _position);
        if (action == UndoAction.Merge && _position > 0)
            _history[_position - 1] = _history[_position - 1] with { After = after };
        else
        {
            _history.Add(new(before, after));
            _position++;
        }
        if (undoLimit > 0 && _history.Count > undoLimit)
        {
            var removed = _history.Count - undoLimit;
            _history.RemoveRange(0, removed);
            _position -= removed;
        }
    }

    private bool RestoreSnapshot(Snapshot snapshot, string text)
    {
        if (snapshot.Text != text) return false;
        _text = text;
        _spans = snapshot.Spans.ToList();
        return true;
    }

    private void ApplyChanges(string text, IEnumerable<TextChange> changes)
    {
        // WPF reports offsets in the final buffer. Applying them from left to
        // right puts each remaining span into that change's coordinate space.
        foreach (var change in changes.OrderBy(change => change.Offset))
        {
            if (change.AddedLength == 0 && change.RemovedLength == 0) continue;
            var start = change.Offset;
            var end = start + change.RemovedLength;
            var delta = change.AddedLength - change.RemovedLength;
            _spans = _spans.Select(span => {
                if (span.Start + span.Length <= start)
                {
                    if (span.Start + span.Length == start && start < text.Length && char.IsLetterOrDigit(text[start])) return null;
                    return span;
                }
                return span.Start >= end ? span with { Start = span.Start + delta } : null;
            }).Where(span => span != null).Select(span => span!).ToList();
        }
        _text = text;
        _spans.RemoveAll(span => span.Start < 0 || span.Start + span.Length > text.Length ||
            text.Substring(span.Start, span.Length) != span.Label);
    }

    internal void Insert(int start, string label, string token)
    {
        _spans.RemoveAll(s => s.Start < start + label.Length && s.Start + s.Length > start);
        _spans.Add(new(start, label.Length, label, token));
        _spans.Sort((a, b) => a.Start.CompareTo(b.Start));
        // The native TextChanged event closes before the completion callback
        // attaches its identity. Add that identity to the same undo unit.
        if (_position > 0 && _position == _history.Count)
            _history[_position - 1] = _history[_position - 1] with { After = Capture() };
    }

    internal string WireText(string text)
    {
        foreach (var span in _spans.AsEnumerable().Reverse())
            if (span.Start >= 0 && span.Start + span.Length <= text.Length && text.Substring(span.Start, span.Length) == span.Label)
                text = text.Remove(span.Start, span.Length).Insert(span.Start, span.Token);
        return text;
    }
}
