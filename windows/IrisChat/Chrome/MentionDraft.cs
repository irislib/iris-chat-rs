using System;
using System.Collections.Generic;
using System.Linq;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

/// UTF-16 ranges track selected identities independently of their displayed name.
internal sealed class MentionDraft
{
    private sealed record Span(int Start, int Length, string Label, string Token);
    private List<Span> _spans = new();
    private string _text = "";

    internal string Restore(string wire)
    {
        _spans.Clear();
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

    internal void Changed(string text)
    {
        if (text == _text) return;
        var prefix = 0;
        while (prefix < Math.Min(_text.Length, text.Length) && _text[prefix] == text[prefix]) prefix++;
        var suffix = 0;
        while (suffix < Math.Min(_text.Length, text.Length) - prefix && _text[^(suffix + 1)] == text[^(suffix + 1)]) suffix++;
        var end = _text.Length - suffix;
        var delta = text.Length - _text.Length;
        _spans = _spans.Where(s => !(s.Start + s.Length == prefix && prefix < text.Length && char.IsLetterOrDigit(text[prefix])))
            .Select(s => s.Start + s.Length <= prefix ? s : s.Start >= end ? s with { Start = s.Start + delta } : null)
            .Where(s => s != null).Select(s => s!).ToList();
        _text = text;
    }

    internal void Insert(int start, string label, string token)
    {
        _spans.RemoveAll(s => s.Start < start + label.Length && s.Start + s.Length > start);
        _spans.Add(new(start, label.Length, label, token));
        _spans.Sort((a, b) => a.Start.CompareTo(b.Start));
    }

    internal string WireText(string text)
    {
        foreach (var span in _spans.AsEnumerable().Reverse())
            if (span.Start >= 0 && span.Start + span.Length <= text.Length && text.Substring(span.Start, span.Length) == span.Label)
                text = text.Remove(span.Start, span.Length).Insert(span.Start, span.Token);
        return text;
    }
}
