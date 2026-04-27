using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

public partial class ChatRow : UserControl
{
    public static readonly DependencyProperty ChatProperty =
        DependencyProperty.Register(nameof(Chat), typeof(ChatThreadSnapshot), typeof(ChatRow),
            new PropertyMetadata(null, (d, _) => ((ChatRow)d).Refresh()));

    public static readonly DependencyProperty IsActiveProperty =
        DependencyProperty.Register(nameof(IsActive), typeof(bool), typeof(ChatRow),
            new PropertyMetadata(false, (d, _) => ((ChatRow)d).RefreshActive()));

    public ChatThreadSnapshot? Chat
    {
        get => (ChatThreadSnapshot?)GetValue(ChatProperty);
        set => SetValue(ChatProperty, value);
    }

    public bool IsActive
    {
        get => (bool)GetValue(IsActiveProperty);
        set => SetValue(IsActiveProperty, value);
    }

    public event Action<ChatThreadSnapshot>? Activated;

    public ChatRow()
    {
        InitializeComponent();
        MouseLeftButtonUp += OnClick;
        MouseEnter += (_, _) => RefreshActive();
        MouseLeave += (_, _) => RefreshActive();
    }

    private void OnClick(object sender, MouseButtonEventArgs e)
    {
        if (Chat is { } c) Activated?.Invoke(c);
    }

    private void Refresh()
    {
        var chat = Chat;
        if (chat == null) return;

        AvatarView.Label = string.IsNullOrEmpty(chat.displayName)
            ? chat.chatId
            : chat.displayName;
        AvatarView.PictureUrl = chat.pictureUrl;

        NameText.Text = string.IsNullOrEmpty(chat.displayName)
            ? chat.chatId.Substring(0, Math.Min(10, chat.chatId.Length))
            : chat.displayName;

        if (chat.lastMessageAtSecs is { } secs && secs > 0)
        {
            var t = DateTimeOffset.FromUnixTimeSeconds((long)secs).LocalDateTime;
            TimeText.Text = (DateTime.Now - t) < TimeSpan.FromHours(24)
                ? t.ToString("HH:mm")
                : t.ToString("MMM d");
        }
        else
        {
            TimeText.Text = string.Empty;
        }

        var preview = chat.lastMessagePreview ?? string.Empty;
        if (chat.isTyping) preview = "typing…";
        PreviewText.Text = preview;

        if (chat.unreadCount > 0)
        {
            UnreadBadge.Visibility = Visibility.Visible;
            UnreadText.Text = chat.unreadCount > 99 ? "99+" : chat.unreadCount.ToString();
        }
        else
        {
            UnreadBadge.Visibility = Visibility.Collapsed;
        }

        RefreshActive();
    }

    private void RefreshActive()
    {
        if (IsActive)
        {
            RowBorder.Background = (System.Windows.Media.Brush)FindResource("Panel");
        }
        else if (IsMouseOver)
        {
            RowBorder.Background = (System.Windows.Media.Brush)FindResource("Panel");
            RowBorder.Opacity = 0.7;
        }
        else
        {
            RowBorder.Background = System.Windows.Media.Brushes.Transparent;
            RowBorder.Opacity = 1.0;
        }
    }
}
