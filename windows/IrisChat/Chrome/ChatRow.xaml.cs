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
        KeyDown += (_, e) =>
        {
            if (Keyboard.Modifiers != ModifierKeys.None || e.Key is not (Key.Enter or Key.Space)) return;
            if (Chat is { } chat) Activated?.Invoke(chat);
            e.Handled = true;
        };
        MouseEnter += (_, _) => RefreshActive();
        MouseLeave += (_, _) => RefreshActive();
    }

    private void OnClick(object sender, MouseButtonEventArgs e)
    {
        Focus();
        if (Chat is { } c) Activated?.Invoke(c);
    }

    private void Refresh()
    {
        var chat = Chat;
        if (chat == null) return;
        Uid = "chat:" + chat.chatId;
        System.Windows.Automation.AutomationProperties.SetName(this, chat.displayName);

        AvatarView.SocialConnection = chat.socialConnection;
        AvatarView.OwnerPubkeyHex = chat.kind == ChatKind.Direct ? chat.chatId : null;
        AvatarView.Label = string.IsNullOrEmpty(chat.displayName)
            ? "Iris user"
            : chat.displayName;
        AvatarView.PictureUrl = chat.pictureUrl;

        var name = PersonNames.Present(chat.displayName, chat.kind == ChatKind.Direct ? chat.chatId : null,
            PersonNames.Explicit(chat.nickname, chat.profileName));
        NameText.Text = name.name;
        NameText.FontStyle = PersonNames.Style(name);
        MutedBellText.Visibility = chat.isMuted ? Visibility.Visible : Visibility.Collapsed;
        PinnedText.Visibility = chat.isPinned ? Visibility.Visible : Visibility.Collapsed;

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
