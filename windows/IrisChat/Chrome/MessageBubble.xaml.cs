using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

public partial class MessageBubble : UserControl
{
    private static readonly string[] DefaultReactionEmojis = ["❤️", "👍", "😂", "😮", "😢", "🙏", "🔥"];
    private static readonly List<string> RecentReactionEmojis = LoadRecentReactionEmojis();

    /// Snapshot of the recent-reactions cache so other chrome (composer
    /// emoji picker) can show the same "Recent" list without owning state.
    public static IReadOnlyList<string> RecentReactionEmojiSnapshot() =>
        RecentReactionEmojis.ToArray();

    public static void RememberEmojiUsage(string emoji) => RememberReactionEmoji(emoji);
    private static readonly ConcurrentDictionary<string, ImageSource> AttachmentImageCache = new();
    private const int RecentReactionEmojiLimit = 16;
    private const int AttachmentPreviewDecodeWidth = 640;
    private const double CollapsedBodyMaxHeight = 320;
    private const int LongBodyCharThreshold = 800;
    private const int LongBodyNewlineThreshold = 14;

    private ChatMessageSnapshot? _message;
    private bool _bodyExpanded;

    public MessageBubble()
    {
        InitializeComponent();
    }

    public void Bind(ChatMessageSnapshot message, bool showAuthor = false, string? authorLabel = null)
    {
        _message = message;
        BuildContextMenu(message);

        if (showAuthor && !string.IsNullOrEmpty(authorLabel))
        {
            AuthorText.Text = authorLabel;
            AuthorText.Visibility = Visibility.Visible;
        }
        else
        {
            AuthorText.Visibility = Visibility.Collapsed;
        }

        BodyText.Text = message.body ?? string.Empty;
        BodyText.Visibility = string.IsNullOrEmpty(message.body) ? Visibility.Collapsed : Visibility.Visible;
        ApplyJumbomojiFont(message.body);
        ApplyBodyTruncation(message.body);

        if (message.kind == ChatMessageKind.System)
        {
            Bubble.Background = (Brush)FindResource("Panel");
            Bubble.HorizontalAlignment = HorizontalAlignment.Center;
            BodyText.Foreground = (Brush)FindResource("TextMuted");
            BodyText.HorizontalAlignment = HorizontalAlignment.Center;
            DeliveryText.Visibility = Visibility.Collapsed;
        }
        else if (message.isOutgoing)
        {
            Bubble.Background = (Brush)FindResource("BubbleMine");
            Bubble.HorizontalAlignment = HorizontalAlignment.Right;
            BodyText.Foreground = Brushes.White;
            DeliveryText.Visibility = Visibility.Visible;
        }
        else
        {
            Bubble.Background = (Brush)FindResource("BubbleTheirs");
            Bubble.HorizontalAlignment = HorizontalAlignment.Left;
            BodyText.Foreground = Brushes.White;
            DeliveryText.Visibility = Visibility.Collapsed;
        }

        TimeText.Text = DateTimeOffset.FromUnixTimeSeconds((long)message.createdAtSecs)
            .LocalDateTime.ToString("HH:mm");
        DeliveryText.Text = message.delivery switch
        {
            DeliveryState.Queued or DeliveryState.Pending or DeliveryState.Sent => "✓",
            DeliveryState.Received or DeliveryState.Seen => "✓✓",
            DeliveryState.Failed => "!",
            _ => string.Empty,
        };

        if (message.attachments != null && message.attachments.Length > 0)
        {
            AttachmentsList.ItemsSource = null;
            var panel = new StackPanel();
            foreach (var att in message.attachments)
            {
                panel.Children.Add(BuildAttachmentRow(att));
            }
            AttachmentsList.ItemsSource = panel.Children;
            AttachmentsList.Visibility = Visibility.Visible;
        }
        else
        {
            AttachmentsList.Visibility = Visibility.Collapsed;
        }
    }

    private void BuildContextMenu(ChatMessageSnapshot message)
    {
        var menu = new ContextMenu();
        if (message.kind != ChatMessageKind.System)
        {
            var react = new MenuItem { Header = "React" };
            foreach (var emoji in ReactionPickerEmojis())
            {
                var selected = emoji;
                var item = new MenuItem { Header = selected };
                item.Click += (_, _) => ToggleReaction(selected);
                react.Items.Add(item);
            }
            react.Items.Add(new Separator());
            var more = new MenuItem { Header = "More emoji…" };
            more.Click += (_, _) => OpenReactionPicker();
            react.Items.Add(more);
            menu.Items.Add(react);
            menu.Items.Add(new Separator());
        }

        var forward = new MenuItem { Header = "Forward" };
        forward.Click += OnForwardMessage;
        menu.Items.Add(forward);

        var copy = new MenuItem { Header = "Copy text" };
        copy.Click += OnCopyText;
        menu.Items.Add(copy);

        var info = new MenuItem { Header = "Info" };
        info.Click += OnShowInfo;
        menu.Items.Add(info);

        ContextMenu = menu;
    }

    private void OpenReactionPicker()
    {
        if (_message == null) return;
        var picker = new EmojiPicker
        {
            RecentEmojis = RecentReactionEmojiSnapshot(),
            MessageEmojis = MessageReactionEmojis(_message),
        };
        var popup = new System.Windows.Controls.Primitives.Popup
        {
            PlacementTarget = Bubble,
            Placement = System.Windows.Controls.Primitives.PlacementMode.Top,
            StaysOpen = false,
            AllowsTransparency = true,
            PopupAnimation = System.Windows.Controls.Primitives.PopupAnimation.Fade,
        };
        var border = new Border
        {
            Background = (Brush)FindResource("Background"),
            BorderBrush = (Brush)FindResource("Border"),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(10),
            Child = picker,
        };
        popup.Child = border;
        picker.EmojiPicked += emoji =>
        {
            ToggleReaction(emoji);
            popup.IsOpen = false;
        };
        popup.IsOpen = true;
    }

    private static IEnumerable<string> ReactionPickerEmojis() => DefaultReactionEmojis;

    private static IReadOnlyList<string> MessageReactionEmojis(ChatMessageSnapshot message) =>
        UniqueReactionEmojis((message.reactions ?? []).Select(reaction => reaction.emoji)).ToArray();

    private static IEnumerable<string> UniqueReactionEmojis(IEnumerable<string> emojis)
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

    private static void RememberReactionEmoji(string emoji)
    {
        var trimmed = emoji.Trim();
        if (trimmed.Length == 0) return;
        RecentReactionEmojis.Remove(trimmed);
        RecentReactionEmojis.Insert(0, trimmed);
        if (RecentReactionEmojis.Count > RecentReactionEmojiLimit)
        {
            RecentReactionEmojis.RemoveRange(RecentReactionEmojiLimit, RecentReactionEmojis.Count - RecentReactionEmojiLimit);
        }
        SaveRecentReactionEmojis();
    }

    private static List<string> LoadRecentReactionEmojis()
    {
        var path = RecentReactionEmojiPath();
        if (path == null || !File.Exists(path)) return [];
        try
        {
            return UniqueReactionEmojis(File.ReadAllLines(path)).Take(RecentReactionEmojiLimit).ToList();
        }
        catch
        {
            return [];
        }
    }

    private static void SaveRecentReactionEmojis()
    {
        var path = RecentReactionEmojiPath();
        if (path == null) return;
        try
        {
            var directory = Path.GetDirectoryName(path);
            if (!string.IsNullOrEmpty(directory))
            {
                Directory.CreateDirectory(directory);
            }
            File.WriteAllLines(path, RecentReactionEmojis);
        }
        catch
        {
            // Recent reactions are a convenience cache.
        }
    }

    private static string? RecentReactionEmojiPath()
    {
        var appData = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
        return string.IsNullOrEmpty(appData)
            ? null
            : Path.Combine(appData, "IrisChat", "recent-reactions.txt");
    }

    private void ToggleReaction(string emoji)
    {
        if (_message == null) return;
        RememberReactionEmoji(emoji);
        App.CurrentManager.ToggleReaction(_message.chatId, _message.id, emoji);
    }

    private FrameworkElement BuildAttachmentRow(MessageAttachmentSnapshot att)
    {
        if (att.isImage)
        {
            var image = new System.Windows.Controls.Image
            {
                Width = 320,
                MaxHeight = 280,
                Stretch = Stretch.Uniform,
                Margin = new Thickness(0, 4, 0, 0),
            };
            _ = LoadImageAsync(att, image);

            var btn = new Button
            {
                Style = (Style)FindResource("GhostButton"),
                Padding = new Thickness(0),
                Background = Brushes.Transparent,
                BorderBrush = Brushes.Transparent,
                Content = image,
                Margin = new Thickness(0, 0, 0, 4),
            };
            btn.Click += (_, _) => OpenAttachment(att);
            btn.ContextMenu = BuildAttachmentContextMenu(att);
            return btn;
        }

        var fileBtn = new Button
        {
            Style = (Style)FindResource("SecondaryButton"),
            Margin = new Thickness(0, 4, 0, 0),
            HorizontalContentAlignment = HorizontalAlignment.Left,
            Content = new StackPanel
            {
                Orientation = Orientation.Horizontal,
                Children =
                {
                    new TextBlock { Text = "📎 ", FontSize = 14, VerticalAlignment = VerticalAlignment.Center },
                    new TextBlock
                    {
                        Text = string.IsNullOrEmpty(att.filename) ? "Attachment" : att.filename,
                        Foreground = (Brush)FindResource("TextPrimary"),
                        FontSize = 13,
                        VerticalAlignment = VerticalAlignment.Center,
                    },
                },
            },
        };
        fileBtn.Click += (_, _) => OpenAttachment(att);
        fileBtn.ContextMenu = BuildAttachmentContextMenu(att);
        return fileBtn;
    }

    private ContextMenu BuildAttachmentContextMenu(MessageAttachmentSnapshot attachment)
    {
        var menu = new ContextMenu();
        var forward = new MenuItem { Header = "Forward" };
        forward.Click += (_, _) => PresentForwardPicker(ForwardableAttachmentText(attachment));
        menu.Items.Add(forward);

        var copy = new MenuItem { Header = "Copy link" };
        copy.Click += (_, _) =>
        {
            try { Clipboard.SetText(attachment.htreeUrl ?? string.Empty); }
            catch { /* clipboard contention */ }
        };
        menu.Items.Add(copy);
        return menu;
    }

    private static async Task LoadImageAsync(MessageAttachmentSnapshot att, System.Windows.Controls.Image control)
    {
        if (Application.Current is not App app || app.Manager == null) return;

        var cacheKey = string.IsNullOrWhiteSpace(att.htreeUrl) ? $"{att.nhash}:{att.filename}" : att.htreeUrl;
        if (AttachmentImageCache.TryGetValue(cacheKey, out var cached))
        {
            SetImageSource(control, cached);
            return;
        }

        var data = await app.Manager.DownloadAttachmentAsync(att).ConfigureAwait(false);
        if (data == null) return;

        var decoded = await Task.Run(() => DecodeAttachmentPreview(data)).ConfigureAwait(false);
        if (decoded == null) return;

        AttachmentImageCache[cacheKey] = decoded;
        SetImageSource(control, decoded);
    }

    private static BitmapImage? DecodeAttachmentPreview(byte[] data)
    {
        try
        {
            using var ms = new System.IO.MemoryStream(data);
            var bmp = new BitmapImage();
            bmp.BeginInit();
            bmp.CacheOption = BitmapCacheOption.OnLoad;
            bmp.DecodePixelWidth = AttachmentPreviewDecodeWidth;
            bmp.StreamSource = ms;
            bmp.EndInit();
            bmp.Freeze();
            return bmp;
        }
        catch
        {
            return null;
        }
    }

    private static void SetImageSource(System.Windows.Controls.Image control, ImageSource source)
    {
        if (control.Dispatcher.CheckAccess())
        {
            control.Source = source;
        }
        else
        {
            _ = control.Dispatcher.InvokeAsync(() =>
            {
                control.Source = source;
            });
        }
    }

    private static void OpenAttachment(MessageAttachmentSnapshot att)
    {
        if (Application.Current is App app && app.Manager != null)
        {
            _ = app.Manager.OpenAttachmentAsync(att);
        }
    }

    private void ApplyBodyTruncation(string? body)
    {
        _bodyExpanded = false;
        var text = body ?? string.Empty;
        var needsTruncation = text.Length > LongBodyCharThreshold;
        if (!needsTruncation)
        {
            var newlines = 0;
            foreach (var c in text)
            {
                if (c == '\n' && ++newlines >= LongBodyNewlineThreshold)
                {
                    needsTruncation = true;
                    break;
                }
            }
        }

        if (needsTruncation)
        {
            BodyText.MaxHeight = CollapsedBodyMaxHeight;
            ShowMoreLink.Visibility = Visibility.Visible;
            ShowMoreLink.Text = "Show more";
        }
        else
        {
            BodyText.MaxHeight = double.PositiveInfinity;
            ShowMoreLink.Visibility = Visibility.Collapsed;
        }
    }

    private void ApplyJumbomojiFont(string? body)
    {
        BodyText.FontSize = JumbomojiCount(body ?? string.Empty) switch
        {
            1 => 56,
            2 => 48,
            3 => 40,
            4 => 36,
            5 => 32,
            _ => 14,
        };
    }

    private static int JumbomojiCount(string text)
    {
        var trimmed = text.Trim();
        if (trimmed.Length == 0) return 0;

        var count = 0;
        var clusterOpen = false;
        var lastWasJoiner = false;
        var remaining = trimmed.AsSpan();
        while (!remaining.IsEmpty)
        {
            var status = Rune.DecodeFromUtf16(remaining, out var rune, out var consumed);
            if (status != System.Buffers.OperationStatus.Done) return 0;
            remaining = remaining[consumed..];

            var codePoint = rune.Value;
            if (Rune.IsWhiteSpace(rune))
            {
                clusterOpen = false;
                lastWasJoiner = false;
            }
            else if (IsEmojiContinuation(codePoint))
            {
                if (!clusterOpen) return 0;
                lastWasJoiner = codePoint == 0x200D;
            }
            else if (IsEmojiBase(codePoint))
            {
                if (!clusterOpen || !lastWasJoiner)
                {
                    count++;
                    if (count > 5) return 0;
                }
                clusterOpen = true;
                lastWasJoiner = false;
            }
            else
            {
                return 0;
            }
        }

        return count;
    }

    private static bool IsEmojiContinuation(int codePoint) =>
        codePoint == 0x200D ||
        codePoint == 0xFE0F ||
        codePoint is >= 0x1F3FB and <= 0x1F3FF;

    private static bool IsEmojiBase(int codePoint) =>
        codePoint is >= 0x1F000 and <= 0x1FAFF ||
        codePoint is >= 0x2600 and <= 0x27BF;

    private void OnToggleShowMore(object sender, System.Windows.Input.MouseButtonEventArgs e)
    {
        _bodyExpanded = !_bodyExpanded;
        if (_bodyExpanded)
        {
            BodyText.MaxHeight = double.PositiveInfinity;
            ShowMoreLink.Text = "Show less";
        }
        else
        {
            BodyText.MaxHeight = CollapsedBodyMaxHeight;
            ShowMoreLink.Text = "Show more";
        }
        e.Handled = true;
    }

    private void OnCopyText(object sender, RoutedEventArgs e)
    {
        if (_message == null) return;
        var pieces = new List<string>();
        if (!string.IsNullOrEmpty(_message.body)) pieces.Add(_message.body);
        if (_message.attachments != null)
        {
            foreach (var att in _message.attachments)
            {
                pieces.Add(att.htreeUrl);
            }
        }
        if (pieces.Count == 0) return;
        try { Clipboard.SetText(string.Join("\n", pieces)); }
        catch { /* clipboard contention */ }
    }

    private void OnForwardMessage(object sender, RoutedEventArgs e)
    {
        if (_message == null) return;
        PresentForwardPicker(ForwardableMessageText(_message));
    }

    private static string ForwardableMessageText(ChatMessageSnapshot message)
    {
        var pieces = new List<string>();
        var body = ReplyStrippedBody(message.body ?? string.Empty).Trim();
        if (!string.IsNullOrEmpty(body)) pieces.Add(body);
        if (message.attachments != null)
        {
            pieces.AddRange(message.attachments
                .Select(ForwardableAttachmentText)
                .Where(s => !string.IsNullOrEmpty(s)));
        }
        return string.Join("\n", pieces);
    }

    private static string ForwardableAttachmentText(MessageAttachmentSnapshot attachment) =>
        (attachment.htreeUrl ?? string.Empty).Trim();

    private static string ReplyStrippedBody(string body)
    {
        const string prefix = "↩ ";
        if (!body.StartsWith(prefix, StringComparison.Ordinal)) return body;
        var separator = body.IndexOf("\n\n", StringComparison.Ordinal);
        if (separator < 0) return body;
        var header = body.Substring(prefix.Length, separator - prefix.Length);
        return header.Contains(':') ? body[(separator + 2)..] : body;
    }

    private void PresentForwardPicker(string text)
    {
        var trimmed = text.Trim();
        if (string.IsNullOrEmpty(trimmed)) return;

        var chats = App.CurrentManager.ChatList;
        var owner = Window.GetWindow(this);
        var window = new Window
        {
            Title = "Forward",
            Owner = owner,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            Width = 360,
            Height = 480,
            MinWidth = 320,
            MinHeight = 360,
            Background = (Brush)FindResource("Background"),
        };

        var root = new DockPanel { Margin = new Thickness(16) };
        var actions = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
            Margin = new Thickness(0, 12, 0, 0),
        };
        var cancel = new Button
        {
            Content = "Cancel",
            Style = (Style)FindResource("GhostButton"),
            Margin = new Thickness(0, 0, 8, 0),
        };
        var send = new Button
        {
            Content = "Send",
            Style = (Style)FindResource("PrimaryButton"),
            IsEnabled = false,
        };
        actions.Children.Add(cancel);
        actions.Children.Add(send);
        DockPanel.SetDock(actions, Dock.Bottom);
        root.Children.Add(actions);

        var list = new ListBox
        {
            SelectionMode = SelectionMode.Multiple,
            ItemsSource = chats,
            DisplayMemberPath = "displayName",
            Background = Brushes.Transparent,
            BorderThickness = new Thickness(0),
        };
        list.SelectionChanged += (_, _) => send.IsEnabled = list.SelectedItems.Count > 0;
        root.Children.Add(list);

        cancel.Click += (_, _) => window.Close();
        send.Click += (_, _) =>
        {
            var targets = list.SelectedItems.Cast<ChatThreadSnapshot>().ToArray();
            if (targets.Length == 0) return;
            foreach (var chat in targets)
            {
                App.CurrentManager.SendMessage(chat.chatId, trimmed);
            }
            App.CurrentManager.OpenChat(targets[0].chatId);
            window.Close();
        };

        window.Content = root;
        window.ShowDialog();
    }

    private void OnShowInfo(object sender, RoutedEventArgs e)
    {
        if (_message == null) return;
        var owner = Window.GetWindow(this);
        var window = new MessageInfoWindow(_message)
        {
            Owner = owner,
        };
        window.ShowDialog();
    }
}
