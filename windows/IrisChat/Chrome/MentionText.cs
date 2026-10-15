using System;
using System.Linq;
using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Data;
using System.Windows.Input;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

internal static class MentionText
{
    internal static string Label(MessageMention mention)
    {
        var name = PersonNames.Explicit(mention.ownerPubkeyHex)
            ?? App.CurrentManager.CurrentChat?.participants.FirstOrDefault(p => p.ownerPubkeyHex == mention.ownerPubkeyHex)?.displayName;
        return string.IsNullOrWhiteSpace(name) ? mention.label : "@" + name;
    }

    internal static void Render(RichTextBox view, string text)
    {
        var paragraph = new Paragraph { Margin = new Thickness(0) };
        var cursor = 0;
        foreach (var mention in Native.MessageMentions(text))
        {
            var start = (int)mention.start;
            AppendLinks(view, paragraph, text[cursor..start]);
            var label = Label(mention);
            var link = new Hyperlink(new Run(label)) { Cursor = Cursors.Hand, FontWeight = FontWeights.SemiBold };
            link.SetBinding(TextElement.ForegroundProperty, new Binding(nameof(Control.Foreground)) { Source = view });
            link.Click += (_, _) => PeerProfileWindow.ShowProfile(Window.GetWindow(view), App.CurrentManager, mention.ownerPubkeyHex, label.TrimStart('@'));
            paragraph.Inlines.Add(link);
            cursor = start + (int)mention.length;
        }
        AppendLinks(view, paragraph, text[cursor..]);
        view.IsTabStop = paragraph.Inlines.OfType<Hyperlink>().Any();
        view.Document = new FlowDocument(paragraph) { PagePadding = new Thickness(0) };
    }

    private static readonly Regex Links = new(@"(?i)(?:^|(?<=[\s(\[{<]))(?:https?://|www\.)[^\s<]+", RegexOptions.Compiled, TimeSpan.FromMilliseconds(100));
    private static void AppendLinks(RichTextBox view, Paragraph paragraph, string text)
    {
        var cursor = 0;
        foreach (Match match in Links.Matches(text))
        {
            var visible = match.Value.TrimEnd('.', ',', ';', ':', '!', '?', ')', ']');
            if (!Uri.TryCreate(visible.StartsWith("www.", StringComparison.OrdinalIgnoreCase) ? "https://" + visible : visible, UriKind.Absolute, out var url)
                || (url.Scheme != "http" && url.Scheme != "https")) continue;
            paragraph.Inlines.Add(new Run(text[cursor..match.Index]));
            var link = new Hyperlink(new Run(visible)) { Cursor = Cursors.Hand };
            link.SetBinding(TextElement.ForegroundProperty, new Binding(nameof(Control.Foreground)) { Source = view });
            link.Click += (_, _) => PlatformDocumentOpener.OpenUrl(url.AbsoluteUri);
            paragraph.Inlines.Add(link);
            cursor = match.Index + visible.Length;
        }
        paragraph.Inlines.Add(new Run(text[cursor..]));
    }
}
