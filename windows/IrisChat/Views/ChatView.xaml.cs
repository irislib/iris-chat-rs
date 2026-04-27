using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using IrisChat.Bindings;
using IrisChat.Chrome;

namespace IrisChat.Views;

public partial class ChatView : UserControl
{
    public string? ChatId { get; set; }

    private string? _focusedChatId;

    public ChatView()
    {
        InitializeComponent();
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

    private void Refresh()
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null || (ChatId != null && chat.chatId != ChatId)) return;

        if (_focusedChatId != chat.chatId)
        {
            _focusedChatId = chat.chatId;
            Dispatcher.BeginInvoke(new Action(() => Composer.FocusInput()));
        }

        HeaderTitle.Text = chat.displayName;
        HeaderSubtitle.Text = chat.subtitle ?? string.Empty;
        HeaderAvatar.Label = chat.displayName;
        HeaderAvatar.PictureUrl = chat.pictureUrl;
        GroupDetailsButton.Visibility = chat.kind == ChatKind.Group ? Visibility.Visible : Visibility.Collapsed;

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

        // Messages
        MessagesList.Items.Clear();
        var isGroup = chat.kind == ChatKind.Group;
        ChatMessageSnapshot? prev = null;
        foreach (var m in chat.messages ?? Array.Empty<ChatMessageSnapshot>())
        {
            var bubble = new MessageBubble();
            var showAuthor = isGroup && !m.isOutgoing && (prev == null || prev.author != m.author);
            bubble.Bind(m, showAuthor, AuthorLabel(m.author));
            MessagesList.Items.Add(bubble);
            prev = m;
        }

        Dispatcher.BeginInvoke(new Action(() => ScrollHost.ScrollToBottom()));

        // Mark as seen
        var unread = (chat.messages ?? Array.Empty<ChatMessageSnapshot>())
            .Where(m => !m.isOutgoing)
            .Select(m => m.id)
            .ToArray();
        if (unread.Length > 0)
        {
            App.CurrentManager.MarkMessagesSeen(chat.chatId, unread);
        }
    }

    private string AuthorLabel(string pubkeyHex)
    {
        // Best-effort: shorten the hex pubkey. Group member display names are
        // available via group_details when on the group's members tab.
        if (string.IsNullOrEmpty(pubkeyHex)) return string.Empty;
        return pubkeyHex.Length <= 10 ? pubkeyHex : $"{pubkeyHex.Substring(0, 6)}…{pubkeyHex.Substring(pubkeyHex.Length - 4)}";
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
}
