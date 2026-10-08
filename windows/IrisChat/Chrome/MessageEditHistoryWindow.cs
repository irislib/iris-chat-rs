using System;
using System.ComponentModel;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Threading;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

public sealed class MessageEditHistoryWindow : Window
{
    private readonly AppManager _manager;
    private readonly string? _accountAtOpen;
    private readonly string _chatId;
    private readonly string _messageId;
    private readonly StackPanel _versions = new() { Margin = new Thickness(20) };
    private readonly DispatcherTimer _expiryTimer = new();
    private MessageEditSnapshot[]? _rendered;

    public MessageEditHistoryWindow(ChatMessageSnapshot target)
    {
        _manager = App.CurrentManager;
        _accountAtOpen = _manager.Account?.publicKeyHex;
        _chatId = target.chatId;
        _messageId = target.id;
        Title = "Edit history";
        Width = 440;
        Height = 520;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        ShowInTaskbar = false;
        Background = (Brush)FindResource("Background");
        Content = new ScrollViewer
        {
            Content = _versions,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        };
        _expiryTimer.Tick += (_, _) => RefreshHistory();
        Loaded += (_, _) =>
        {
            _manager.PropertyChanged += OnStateChanged;
            RefreshHistory();
        };
        Closed += (_, _) =>
        {
            _manager.PropertyChanged -= OnStateChanged;
            _expiryTimer.Stop();
        };
    }

    internal static bool CanShow(ChatMessageSnapshot message) =>
        !message.deletedForEveryone && message.editHistory is { Length: > 0 } &&
        (!message.expiresAtSecs.HasValue || message.expiresAtSecs.Value > (ulong)DateTimeOffset.UtcNow.ToUnixTimeSeconds());

    private void OnStateChanged(object? sender, PropertyChangedEventArgs e) => RefreshHistory();

    private void RefreshHistory()
    {
        _expiryTimer.Stop();
        var chat = _manager.CurrentChat;
        var message = chat?.chatId == _chatId ? chat.messages.FirstOrDefault(m => m.id == _messageId) : null;
        if (_accountAtOpen == null || _manager.Account?.publicKeyHex != _accountAtOpen ||
            _manager.ActiveScreen is not Screen.Chat { chatId: var activeChatId } || activeChatId != _chatId ||
            message == null || !CanShow(message))
        {
            Close();
            return;
        }
        if (message.expiresAtSecs is { } expiry)
        {
            var remaining = (double)expiry - DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() / 1000.0;
            _expiryTimer.Interval = TimeSpan.FromSeconds(Math.Clamp(remaining, 0.01, 86_400));
            _expiryTimer.Start();
        }
        if (_rendered != null && _rendered.SequenceEqual(message.editHistory)) return;
        _rendered = message.editHistory;
        _versions.Children.Clear();
        for (var i = message.editHistory.Length - 1; i >= 0; i--)
        {
            var version = message.editHistory[i];
            var current = i == message.editHistory.Length - 1;
            _versions.Children.Add(new TextBlock
            {
                Text = current ? "Current" : i == 0 ? "Original" : $"Edit {i}",
                FontWeight = FontWeights.SemiBold,
                Foreground = (Brush)FindResource("TextPrimary"),
                Margin = new Thickness(0, current ? 0 : 20, 0, 4),
            });
            _versions.Children.Add(new TextBlock
            {
                Text = DateTimeOffset.FromUnixTimeSeconds((long)Math.Min(version.createdAtSecs, 253402300799UL)).LocalDateTime.ToString("g"),
                Foreground = (Brush)FindResource("TextMuted"),
                FontSize = 11,
                Margin = new Thickness(0, 0, 0, 6),
            });
            _versions.Children.Add(new TextBox
            {
                Text = ReadableBody(version.body),
                IsReadOnly = true,
                TextWrapping = TextWrapping.Wrap,
                BorderThickness = new Thickness(0),
                Background = Brushes.Transparent,
                Foreground = (Brush)FindResource("TextPrimary"),
                Padding = new Thickness(0),
            });
        }
    }

    internal static string ReadableBody(string text)
    {
        const string prefix = "↩ ";
        if (!text.StartsWith(prefix, StringComparison.Ordinal)) return text;
        var separator = text.IndexOf("\n\n", prefix.Length, StringComparison.Ordinal);
        var authorEnd = text.IndexOf(':', prefix.Length);
        return separator >= 0 && authorEnd > prefix.Length && authorEnd < separator
            ? text[(separator + 2)..] : text;
    }
}
