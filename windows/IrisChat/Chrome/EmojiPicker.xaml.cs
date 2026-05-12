using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace IrisChat.Chrome;

public partial class EmojiPicker : UserControl
{
    public event Action<string>? EmojiPicked;

    /// Optional list shown at the top under "Recent". Caller passes their own
    /// recents store so the picker stays storage-agnostic.
    public IReadOnlyList<string> RecentEmojis { get; set; } = [];

    public IReadOnlyList<string> MessageEmojis { get; set; } = [];

    private static readonly (string Name, string[] Emojis)[] Categories =
    {
        ("Smileys", new[] {
            "😀","😃","😄","😁","😆","😅","😂","🤣","😊","🙂","🙃","😉","😍","🥰","😘","😎","🤩","🥳",
            "😏","😌","😴","😪","🤤","😋","😜","🤪","😝","🤔","🤨","😐","😑","😶","🙄","😬","🤐","🤧",
            "🤒","🤕","😇","🤠","🥺","😢","😭","😠","🤬","🤯","🥶","🥵","😱","😨","😰","😳","🤗"
        }),
        ("Hearts", new[] {
            "❤️","🧡","💛","💚","💙","💜","🖤","🤍","🤎","💖","💗","💓","💞","💕","💘","💝","💟","♥️","💔","❣️"
        }),
        ("Hands", new[] {
            "👍","👎","👌","✌️","🤞","🤟","🤘","🤙","👈","👉","👆","👇","☝️","✋","🤚","🖐","🖖","👋","🤝","🙏","👏","🙌","💪"
        }),
        ("Animals", new[] {
            "🐶","🐱","🐭","🐹","🐰","🦊","🐻","🐼","🐨","🐯","🦁","🐮","🐷","🐸","🐵","🙈","🙉","🙊",
            "🐔","🐧","🐦","🦅","🦉","🦄","🐝","🦋","🐞","🐢","🐍","🦖","🐙","🦀","🐬","🐳","🦈"
        }),
        ("Food", new[] {
            "🍏","🍎","🍐","🍊","🍋","🍌","🍉","🍇","🍓","🫐","🍒","🍑","🥭","🍍","🥥","🥝",
            "🍅","🥑","🥕","🌽","🍆","🥔","🍕","🍔","🍟","🌭","🍿","🥪","🌮","🌯","🍣","🍜","🍝",
            "🍦","🍩","🍪","🎂","🍰","☕","🍵","🍺","🥂","🍷","🥃"
        }),
        ("Activities", new[] {
            "⚽","🏀","🏈","⚾","🥎","🎾","🏐","🏉","🎱","🪀","🏓","🏸","🥅","🏒","🏑","🥍","🏏",
            "🥊","🥋","🎽","⛸","🥌","🛷","🪂","🏋","🤸","🤺","🏇","⛷","🏂","🏌","🏄","🚣","🏊",
            "🤽","🚴","🚵","🎯","🎮","🎲","🎼","🎤","🎧","🎷","🎸","🥁"
        }),
        ("Travel", new[] {
            "🚗","🚕","🚙","🚌","🚎","🏎","🚓","🚑","🚒","🚐","🛻","🚚","🚛","🚜","🛵","🏍","🛺","🚲","🛴","🛹",
            "🚂","✈️","🚀","🛸","🛶","⛵","🚢","🚁","🗺","🗽","🗼","🏰","🎡","🎢","🎠","🏖","🏝","🏔","🌋","🏕","🌄","🌅","🌌"
        }),
        ("Objects", new[] {
            "📱","💻","⌨","🖥","🖨","🖱","💾","💿","📷","📸","📹","🎥","📺","📻","📞","☎","🔌","🔋","💡","🔦","🕯",
            "💵","💰","💳","💎","⚖","🔧","🔨","🛠","⛏","🪛","🪚","🔩","⚙","🧱","⛓","🧲","🔫","💣","🧨"
        }),
        ("Symbols", new[] {
            "✅","❎","✔","❌","⭕","🚫","⚠","💯","🔥","✨","🌟","⭐","🌈","☀","🌙","⚡","☄","💥","🌊","💧","💦",
            "🎉","🎊","🎁","🎀","🎈","🪅","🎂","🍾","🥇","🥈","🥉","🏆","🎖","🏅","💤","💭","🗯","💬"
        }),
    };

    public EmojiPicker()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            BuildSections(string.Empty);
            SearchInput.Focus();
        };
    }

    private void OnSearchChanged(object sender, TextChangedEventArgs e)
    {
        BuildSections(SearchInput.Text?.Trim() ?? string.Empty);
    }

    private void BuildSections(string query)
    {
        var sections = new List<(string Name, string[] Emojis)>();
        var lower = query.ToLowerInvariant();
        if (string.IsNullOrEmpty(query))
        {
            var messageEmojis = UniqueEmojis(MessageEmojis).ToArray();
            if (messageEmojis.Length > 0)
            {
                sections.Add(("This message", messageEmojis));
            }
            var recent = UniqueEmojis(RecentEmojis).Where(emoji => !messageEmojis.Contains(emoji)).ToArray();
            if (recent.Length > 0)
            {
                sections.Add(("Recent", recent));
            }
            sections.AddRange(Categories);
        }
        else
        {
            foreach (var (name, emojis) in Categories)
            {
                if (name.ToLowerInvariant().Contains(lower))
                {
                    sections.Add((name, emojis));
                    continue;
                }
                var hits = emojis.Where(e => e.Contains(query)).ToArray();
                if (hits.Length > 0)
                {
                    sections.Add((name, hits));
                }
            }
        }

        SectionsHost.Items.Clear();
        foreach (var (name, emojis) in sections)
        {
            SectionsHost.Items.Add(BuildSection(name, emojis));
        }
    }

    private static IEnumerable<string> UniqueEmojis(IEnumerable<string> emojis)
    {
        var seen = new HashSet<string>();
        foreach (var emoji in emojis)
        {
            var trimmed = emoji.Trim();
            if (trimmed.Length > 0 && seen.Add(trimmed))
            {
                yield return trimmed;
            }
        }
    }

    private FrameworkElement BuildSection(string name, IReadOnlyList<string> emojis)
    {
        var stack = new StackPanel { Orientation = Orientation.Vertical };
        var header = new TextBlock
        {
            Text = name,
            FontSize = 11,
            FontWeight = FontWeights.SemiBold,
            Foreground = (Brush)FindResource("TextMuted"),
            Margin = new Thickness(8, 6, 8, 4),
        };
        stack.Children.Add(header);

        var wrap = new WrapPanel { Margin = new Thickness(2, 0, 2, 0) };
        foreach (var emoji in emojis)
        {
            var captured = emoji;
            var btn = new Button
            {
                Content = emoji,
                Style = (Style)FindResource("EmojiButton"),
            };
            btn.Click += (_, _) => EmojiPicked?.Invoke(captured);
            wrap.Children.Add(btn);
        }
        stack.Children.Add(wrap);
        return stack;
    }
}
