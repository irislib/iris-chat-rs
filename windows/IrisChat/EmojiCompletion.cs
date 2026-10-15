using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.RegularExpressions;

namespace IrisChat;

internal sealed record EmojiCompletionToken(int Start, int Length, string Query, bool Closed)
{
    private static readonly Regex Token = new(@"(?:^|\s):([\p{L}\p{N}_+\-]{0,64})(:?)$", RegexOptions.CultureInvariant);

    public static EmojiCompletionToken? Find(string text, int caret, int selectionLength)
    {
        if (selectionLength != 0 || caret < 0 || caret > text.Length ||
            (caret > 0 && caret < text.Length && char.IsSurrogatePair(text, caret - 1))) return null;
        var match = Token.Match(text[..caret]);
        if (!match.Success) return null;
        var query = match.Groups[1];
        var start = query.Index - 1;
        var end = caret;
        var closed = match.Groups[2].Length != 0;
        if (!closed)
        {
            while (end < text.Length && (char.IsLetterOrDigit(text[end]) || "_+-".Contains(text[end])))
            {
                if (end - query.Index >= 64) return null;
                end++;
            }
            if (end < text.Length && text[end] == ':') end++;
        }
        return new(start, end - start, query.Value, closed);
    }
}

internal sealed record EmojiCompletionEntry(string Emoji, string Label, string[] Names)
{
    private static string Normalize(string value) => value.ToLowerInvariant().Replace('-', '_');
    public bool MatchesExactly(string query) => Names.Any(name => Normalize(name) == Normalize(query));

    public static IReadOnlyList<EmojiCompletionEntry> Search(string query, IEnumerable<EmojiCompletionEntry> entries)
    {
        query = Normalize(query);
        if (query.Length < 2) return [];
        return entries.Select((entry, index) => new
        {
            Entry = entry, Index = index,
            Rank = entry.MatchesExactly(query) ? 0 : entry.Names.Any(name => Normalize(name).StartsWith(query, StringComparison.Ordinal)) ? 1
                : entry.Names.Any(name => Normalize(name).Contains(query, StringComparison.Ordinal)) ? 2 : 3,
        }).Where(hit => hit.Rank < 3).OrderBy(hit => hit.Rank).ThenBy(hit => hit.Index).Take(6).Select(hit => hit.Entry).ToArray();
    }
}
