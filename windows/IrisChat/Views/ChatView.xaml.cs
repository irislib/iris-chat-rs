using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using IrisChat.Bindings;
using IrisChat.Chrome;

namespace IrisChat.Views;

public partial class ChatView : UserControl
{
    public string? ChatId { get; set; }

    private string? _focusedChatId;
    private string? _renderedMessageSignature;

    public ChatView()
    {
        InitializeComponent();
        PreviewKeyDown += OnUserActivity;
        PreviewMouseDown += OnUserActivity;
        PreviewMouseMove += OnUserActivity;
        PreviewMouseWheel += OnUserActivity;
        TouchDown += OnUserActivity;
        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            Composer.Submitted += OnSubmit;
            Composer.AttachRequested += OnAttach;
            Composer.Typing += OnTyping;
            Composer.StoppedTyping += OnStoppedTyping;
            Refresh();
        };
        Unloaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged -= OnChanged;
            Composer.Submitted -= OnSubmit;
            Composer.AttachRequested -= OnAttach;
            Composer.Typing -= OnTyping;
            Composer.StoppedTyping -= OnStoppedTyping;
        };
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => Refresh();

    private void OnUserActivity(object sender, InputEventArgs e)
    {
        App.CurrentManager.RecordUserActivity();
    }

    private void Refresh()
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null || (ChatId != null && chat.chatId != ChatId)) return;

        var chatChanged = _focusedChatId != chat.chatId;
        if (chatChanged)
        {
            _focusedChatId = chat.chatId;
            _renderedMessageSignature = null;
            Dispatcher.BeginInvoke(new Action(() => Composer.FocusInput()));
        }

        HeaderTitle.Text = chat.displayName;
        // Header subtitle priority: disappearing-message timeout (clock + ttl)
        // > muted (bell-slash + muted) > group subtitle text. Hide the others
        // when one wins so we don't stack indicators.
        var hasTtl = chat.messageTtlSeconds.HasValue && chat.messageTtlSeconds.Value > 0;
        if (hasTtl)
        {
            HeaderDisappearingText.Text = DisappearingLabel(chat.messageTtlSeconds!.Value);
            HeaderDisappearingStatus.Visibility = Visibility.Visible;
            HeaderMutedStatus.Visibility = Visibility.Collapsed;
            HeaderSubtitle.Visibility = Visibility.Collapsed;
        }
        else if (chat.isMuted)
        {
            HeaderDisappearingStatus.Visibility = Visibility.Collapsed;
            HeaderMutedStatus.Visibility = Visibility.Visible;
            HeaderSubtitle.Visibility = Visibility.Collapsed;
        }
        else
        {
            HeaderDisappearingStatus.Visibility = Visibility.Collapsed;
            HeaderMutedStatus.Visibility = Visibility.Collapsed;
            HeaderSubtitle.Text = chat.subtitle ?? string.Empty;
            HeaderSubtitle.Visibility = Visibility.Visible;
        }
        HeaderAvatar.Label = chat.displayName;
        HeaderAvatar.PictureUrl = chat.pictureUrl;
        MuteChatButton.Visibility = Visibility.Visible;
        MuteChatText.Text = chat.isMuted ? "Unmute chat" : "Mute chat";
        GroupDetailsButton.Visibility = chat.kind == ChatKind.Group ? Visibility.Visible : Visibility.Collapsed;
        DeleteChatButton.Visibility = chat.kind == ChatKind.Direct ? Visibility.Visible : Visibility.Collapsed;

        // Typing indicator
        if (chat.typingIndicators != null && chat.typingIndicators.Length > 0)
        {
            var names = string.Join(", ", chat.typingIndicators.Select(t => t.displayName));
            TypingText.Text = $"{names} typing…";
            TypingText.Visibility = Visibility.Visible;
        }
        else
        {
            TypingText.Visibility = Visibility.Collapsed;
        }

        var messages = chat.messages ?? Array.Empty<ChatMessageSnapshot>();
        var messageSignature = string.Join("|", messages.Select(m =>
            $"{m.id}:{m.delivery}:{m.body}:{m.reactions?.Length ?? 0}:{m.reactors?.Length ?? 0}"));
        var shouldPinToBottom = chatChanged
            || ScrollHost.ScrollableHeight <= 0
            || ScrollHost.VerticalOffset >= ScrollHost.ScrollableHeight - 24;
        if (_renderedMessageSignature != messageSignature)
        {
            _renderedMessageSignature = messageSignature;
            MessagesList.Items.Clear();
            var isGroup = chat.kind == ChatKind.Group;
            ChatMessageSnapshot? prev = null;
            foreach (var m in messages)
            {
                var bubble = new MessageBubble();
                var showAuthor = isGroup && !m.isOutgoing && (prev == null || prev.author != m.author);
                bubble.Bind(m, showAuthor, AuthorLabel(m.author));
                MessagesList.Items.Add(bubble);
                prev = m;
            }

            if (shouldPinToBottom)
            {
                Dispatcher.BeginInvoke(new Action(() => ScrollHost.ScrollToBottom()));
            }
        }

        MarkVisibleMessagesSeen(chat);
    }

    private static void MarkVisibleMessagesSeen(CurrentChatSnapshot chat)
    {
        if (!App.CurrentManager.CanMarkActiveChatSeen) return;

        var messages = chat.messages ?? Array.Empty<ChatMessageSnapshot>();
        var unread = messages
            .Where(m => !m.isOutgoing && m.kind == ChatMessageKind.User && m.delivery != DeliveryState.Seen)
            .Select(m => m.id)
            .ToArray();
        if (unread.Length > 0)
        {
            App.CurrentManager.MarkMessagesSeen(chat.chatId, unread);
        }
    }

    private string AuthorLabel(string pubkeyHex)
    {
        return string.IsNullOrEmpty(pubkeyHex) ? string.Empty : "Iris user";
    }

    private void OnSubmit(string text, IList<string> stagedAttachments)
    {
        var chatId = App.CurrentManager.CurrentChat?.chatId;
        if (string.IsNullOrEmpty(chatId)) return;
        if (stagedAttachments != null && stagedAttachments.Count > 0)
        {
            App.CurrentManager.SendAttachments(chatId, stagedAttachments, text);
            return;
        }
        if (!string.IsNullOrEmpty(text))
        {
            App.CurrentManager.SendMessage(chatId, text);
        }
    }

    private void OnAttach()
    {
        var chatId = App.CurrentManager.CurrentChat?.chatId;
        if (string.IsNullOrEmpty(chatId)) return;
        var files = PlatformFilePicker.PickFiles("Attach files", multiselect: true);
        if (files == null || files.Length == 0) return;
        Composer.AddAttachments(files);
    }

    private void OnTyping()
    {
        var chatId = App.CurrentManager.CurrentChat?.chatId;
        if (!string.IsNullOrEmpty(chatId)) App.CurrentManager.SendTyping(chatId);
    }

    private void OnStoppedTyping()
    {
        var chatId = App.CurrentManager.CurrentChat?.chatId;
        if (!string.IsNullOrEmpty(chatId)) App.CurrentManager.StopTyping(chatId);
    }

    private void OnGroupDetails(object sender, RoutedEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat?.groupId is { } gid)
        {
            App.CurrentManager.Push(new Screen.GroupDetails(gid));
        }
    }

    private void OnHeaderTap(object sender, System.Windows.Input.MouseButtonEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat?.groupId is { } gid)
        {
            App.CurrentManager.Push(new Screen.GroupDetails(gid));
        }
        e.Handled = true;
    }

    private void OnDeleteChat(object sender, RoutedEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null) return;
        App.CurrentManager.DeleteChat(chat.chatId);
    }

    private void OnToggleMute(object sender, RoutedEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null) return;
        App.CurrentManager.SetChatMuted(chat.chatId, !chat.isMuted);
    }

    private static string DisappearingLabel(ulong seconds)
    {
        return seconds switch
        {
            300 => "5 minutes",
            3_600 => "1 hour",
            86_400 => "24 hours",
            604_800 => "1 week",
            2_592_000 => "1 month",
            7_776_000 => "3 months",
            < 3_600 => $"{seconds / 60} min",
            < 86_400 => $"{seconds / 3_600} h",
            < 604_800 => $"{seconds / 86_400} d",
            < 2_592_000 => $"{seconds / 604_800} wk",
            _ => $"{seconds / 2_592_000} mo",
        };
    }
}
