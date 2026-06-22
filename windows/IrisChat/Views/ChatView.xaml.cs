using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Linq;
using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Input;
using System.Windows.Media;
using IrisChat.Bindings;
using IrisChat.Chrome;

namespace IrisChat.Views;

public partial class ChatView : UserControl
{
    private const string IrisSupportEmail = "irismessenger@pm.me";

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

    private void OnUserActivity(object? sender, InputEventArgs e)
    {
        App.CurrentManager.RecordUserActivity();
    }

    private void Refresh()
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null || (ChatId != null && chat.chatId != ChatId)) return;

        var chatChanged = _focusedChatId != chat.chatId;
        var userBlocked = chat.kind == ChatKind.Direct && App.CurrentManager.IsUserBlocked(chat.chatId);
        var messageRequest = chat.kind == ChatKind.Direct && chat.isRequest && !userBlocked;
        if (chatChanged)
        {
            _focusedChatId = chat.chatId;
            _renderedMessageSignature = null;
            if (!userBlocked && !messageRequest)
            {
                Dispatcher.BeginInvoke(new Action(() => Composer.FocusInput()));
            }
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
        BlockedPanel.Visibility = userBlocked ? Visibility.Visible : Visibility.Collapsed;
        MessageRequestPanel.Visibility = messageRequest ? Visibility.Visible : Visibility.Collapsed;
        MessageRequestText.Text = $"Message request from {chat.displayName}";
        Composer.Visibility = userBlocked || messageRequest ? Visibility.Collapsed : Visibility.Visible;

        // Typing indicator
        if (!userBlocked && !messageRequest && chat.typingIndicators != null && chat.typingIndicators.Length > 0)
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
        else if (chat?.kind == ChatKind.Direct)
        {
            ShowDirectChatInfo(chat);
        }
        e.Handled = true;
    }

    private void OnDeleteChat(object sender, RoutedEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null) return;
        App.CurrentManager.DeleteChat(chat.chatId);
    }

    private void OnBlockedDeleteChat(object sender, RoutedEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null) return;
        App.CurrentManager.DeleteChat(chat.chatId);
        App.CurrentManager.NavigateBack();
    }

    private void OnBlockedUnblock(object sender, RoutedEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null) return;
        App.CurrentManager.SetUserBlocked(chat.chatId, false);
    }

    private void OnMessageRequestAccept(object sender, RoutedEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null) return;
        App.CurrentManager.AcceptMessageRequest(chat.chatId);
        Dispatcher.BeginInvoke(new Action(() => Composer.FocusInput()));
    }

    private void OnMessageRequestBlock(object sender, RoutedEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null) return;
        ShowBlockUserDialog(chat.chatId, chat.displayName);
    }

    private void OnMessageRequestReport(object sender, RoutedEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null) return;
        ShowBlockAndReportUserDialog(chat.chatId, chat.displayName);
    }

    private void OnToggleMute(object sender, RoutedEventArgs e)
    {
        var chat = App.CurrentManager.CurrentChat;
        if (chat == null) return;
        App.CurrentManager.SetChatMuted(chat.chatId, !chat.isMuted);
    }

    private void ShowDirectChatInfo(CurrentChatSnapshot chat)
    {
        var window = new Window
        {
            Title = chat.displayName,
            Width = 420,
            Height = 520,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ShowInTaskbar = false,
            ResizeMode = ResizeMode.CanResize,
            Owner = Window.GetWindow(this),
            Background = ResourceBrush("Background"),
        };

        var scroll = new ScrollViewer
        {
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            Padding = new Thickness(20, 18, 20, 18),
        };
        var stack = new StackPanel { Orientation = Orientation.Vertical };
        stack.Children.Add(BuildDirectInfoHeader(chat));
        if (!string.IsNullOrWhiteSpace(chat.about))
        {
            stack.Children.Add(BuildAboutSection(chat.about!));
        }
        var commonGroups = App.CurrentManager.MutualGroups(chat.chatId);
        if (commonGroups.Length > 0)
        {
            stack.Children.Add(BuildCommonGroupsSection(commonGroups, window));
        }

        stack.Children.Add(BuildNicknameSection(chat));
        stack.Children.Add(BuildDirectInfoButton(
            chat.isMuted ? "Unmute chat" : "Mute chat",
            () => App.CurrentManager.SetChatMuted(chat.chatId, !chat.isMuted)
        ));
        var blocked = App.CurrentManager.IsUserBlocked(chat.chatId);
        stack.Children.Add(BuildDirectInfoButton(
            blocked ? "Unblock user" : "Block user",
            () =>
            {
                if (blocked)
                    App.CurrentManager.SetUserBlocked(chat.chatId, false);
                else
                    ShowBlockUserDialog(chat.chatId, chat.displayName);
            },
            destructive: !blocked
        ));
        stack.Children.Add(BuildDirectInfoButton(
            "Report user",
            () => ShowReportUserDialog(chat.chatId, chat.displayName),
            destructive: true
        ));
        stack.Children.Add(BuildDirectInfoButton(
            "Delete chat",
            () =>
            {
                App.CurrentManager.DeleteChat(chat.chatId);
                window.Close();
            },
            destructive: true
        ));

        scroll.Content = stack;
        window.Content = scroll;
        window.ShowDialog();
    }

    private static FrameworkElement BuildDirectInfoHeader(CurrentChatSnapshot chat)
    {
        var row = new Grid { Margin = new Thickness(0, 0, 0, 14) };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        var avatar = new Avatar
        {
            Label = chat.displayName,
            PictureUrl = chat.pictureUrl,
            Size = 64,
            Margin = new Thickness(0, 0, 14, 0),
        };
        Grid.SetColumn(avatar, 0);
        row.Children.Add(avatar);

        var text = new StackPanel
        {
            Orientation = Orientation.Vertical,
            VerticalAlignment = VerticalAlignment.Center,
        };
        text.Children.Add(new TextBlock
        {
            Text = chat.displayName,
            FontSize = 20,
            FontWeight = FontWeights.SemiBold,
            Foreground = ResourceBrush("TextPrimary"),
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        if (!string.IsNullOrWhiteSpace(chat.subtitle))
        {
            text.Children.Add(new TextBlock
            {
                Text = chat.subtitle,
                FontSize = 13,
                Foreground = ResourceBrush("TextMuted"),
                TextTrimming = TextTrimming.CharacterEllipsis,
                Margin = new Thickness(0, 4, 0, 0),
            });
        }
        Grid.SetColumn(text, 1);
        row.Children.Add(text);
        return row;
    }

    private static FrameworkElement BuildAboutSection(string about)
    {
        var border = new Border
        {
            Background = ResourceBrush("Panel"),
            CornerRadius = new CornerRadius(12),
            Padding = new Thickness(14, 12, 14, 12),
            Margin = new Thickness(0, 0, 0, 12),
        };
        var row = new Grid();
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        row.Children.Add(new TextBlock
        {
            Text = "✎",
            FontSize = 18,
            Foreground = ResourceBrush("TextPrimary"),
            Margin = new Thickness(0, 0, 12, 0),
            VerticalAlignment = VerticalAlignment.Top,
        });

        var text = new TextBlock
        {
            TextWrapping = TextWrapping.Wrap,
            TextTrimming = TextTrimming.CharacterEllipsis,
            MaxHeight = 66,
            Foreground = ResourceBrush("TextPrimary"),
        };
        AddLinkHighlightedRuns(text, about.Trim());
        Grid.SetColumn(text, 1);
        row.Children.Add(text);

        border.Child = row;
        return border;
    }

    private static void AddLinkHighlightedRuns(TextBlock block, string text)
    {
        var cursor = 0;
        foreach (Match match in Regex.Matches(text, @"\b(?:https?://|www\.)\S+"))
        {
            if (match.Index > cursor)
            {
                block.Inlines.Add(new Run(text[cursor..match.Index]));
            }
            block.Inlines.Add(new Run(match.Value)
            {
                Foreground = ResourceBrush("Accent"),
                TextDecorations = TextDecorations.Underline,
            });
            cursor = match.Index + match.Length;
        }
        if (cursor < text.Length)
        {
            block.Inlines.Add(new Run(text[cursor..]));
        }
    }

    private static FrameworkElement BuildNicknameSection(CurrentChatSnapshot chat)
    {
        var border = new Border
        {
            Background = ResourceBrush("Panel"),
            CornerRadius = new CornerRadius(12),
            Padding = new Thickness(14, 12, 14, 12),
            Margin = new Thickness(0, 0, 0, 12),
        };
        var stack = new StackPanel { Orientation = Orientation.Vertical };

        var nicknameRow = new Grid();
        nicknameRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        nicknameRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        nicknameRow.Children.Add(new TextBlock
        {
            Text = "Nickname",
            FontWeight = FontWeights.SemiBold,
            Foreground = ResourceBrush("TextPrimary"),
            VerticalAlignment = VerticalAlignment.Center,
        });
        var nickname = chat.nickname?.Trim();
        if (!string.IsNullOrWhiteSpace(nickname))
        {
            var nicknameValue = new TextBlock
            {
                Text = nickname,
                Foreground = ResourceBrush("TextPrimary"),
                TextTrimming = TextTrimming.CharacterEllipsis,
                HorizontalAlignment = HorizontalAlignment.Right,
                VerticalAlignment = VerticalAlignment.Center,
                Margin = new Thickness(12, 0, 0, 0),
            };
            Grid.SetColumn(nicknameValue, 1);
            nicknameRow.Children.Add(nicknameValue);
        }

        var editNickname = new Button
        {
            Background = Brushes.Transparent,
            BorderThickness = new Thickness(0),
            Content = nicknameRow,
            Cursor = Cursors.Hand,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Padding = new Thickness(0),
        };
        editNickname.Click += (_, _) => ShowNicknameEditor(chat);
        stack.Children.Add(editNickname);

        var primaryName = string.IsNullOrWhiteSpace(chat.nickname) ? chat.displayName : chat.nickname;
        if (!string.IsNullOrWhiteSpace(chat.profileName)
            && !string.Equals(chat.profileName.Trim(), primaryName?.Trim(), StringComparison.OrdinalIgnoreCase))
        {
            var profile = new Grid { Margin = new Thickness(0, 10, 0, 0) };
            profile.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            profile.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            profile.Children.Add(new TextBlock
            {
                Text = "Profile name",
                FontWeight = FontWeights.SemiBold,
                Foreground = ResourceBrush("TextPrimary"),
            });
            var value = new TextBlock
            {
                Text = chat.profileName,
                Foreground = ResourceBrush("TextMuted"),
                TextTrimming = TextTrimming.CharacterEllipsis,
                HorizontalAlignment = HorizontalAlignment.Right,
                Margin = new Thickness(12, 0, 0, 0),
            };
            Grid.SetColumn(value, 1);
            profile.Children.Add(value);
            stack.Children.Add(profile);
        }

        border.Child = stack;
        return border;
    }

    private static void ShowNicknameEditor(CurrentChatSnapshot chat)
    {
        var owner = Application.Current.Windows.OfType<Window>().FirstOrDefault(window => window.IsActive)
            ?? Application.Current.MainWindow;
        var dialog = new Window
        {
            Title = "Nickname",
            Width = 360,
            SizeToContent = SizeToContent.Height,
            ResizeMode = ResizeMode.NoResize,
            ShowInTaskbar = false,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            Owner = owner,
            Background = ResourceBrush("Background"),
        };
        var stack = new StackPanel
        {
            Margin = new Thickness(18),
            Orientation = Orientation.Vertical,
        };
        var input = new TextBox
        {
            Text = chat.nickname ?? string.Empty,
            MinWidth = 260,
            Margin = new Thickness(0, 0, 0, 14),
        };
        stack.Children.Add(input);

        var actions = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right,
        };
        var save = new Button
        {
            Content = "Save",
            Padding = new Thickness(12, 7, 12, 7),
            Margin = new Thickness(0, 0, 8, 0),
        };
        save.Click += (_, _) =>
        {
            App.CurrentManager.SetContactNickname(chat.chatId, input.Text ?? string.Empty);
            dialog.Close();
        };
        actions.Children.Add(save);

        if (!string.IsNullOrWhiteSpace(chat.nickname))
        {
            var remove = new Button
            {
                Content = "Remove",
                Padding = new Thickness(12, 7, 12, 7),
            };
            remove.Click += (_, _) =>
            {
                App.CurrentManager.SetContactNickname(chat.chatId, string.Empty);
                dialog.Close();
            };
            actions.Children.Add(remove);
        }
        stack.Children.Add(actions);
        dialog.Content = stack;
        dialog.Loaded += (_, _) =>
        {
            input.Focus();
            input.SelectAll();
        };
        dialog.ShowDialog();
    }

    private static FrameworkElement BuildCommonGroupsSection(
        IEnumerable<ChatThreadSnapshot> groups,
        Window window
    )
    {
        var border = new Border
        {
            Background = ResourceBrush("Panel"),
            CornerRadius = new CornerRadius(12),
            Padding = new Thickness(14, 12, 14, 10),
            Margin = new Thickness(0, 0, 0, 12),
        };
        var stack = new StackPanel { Orientation = Orientation.Vertical };
        stack.Children.Add(new TextBlock
        {
            Text = "Groups in common",
            FontWeight = FontWeights.SemiBold,
            Foreground = ResourceBrush("TextPrimary"),
            Margin = new Thickness(0, 0, 0, 8),
        });

        foreach (var group in groups)
        {
            stack.Children.Add(BuildCommonGroupRow(group, window));
        }

        border.Child = stack;
        return border;
    }

    private static FrameworkElement BuildCommonGroupRow(ChatThreadSnapshot group, Window window)
    {
        var button = new Button
        {
            Style = Application.Current.TryFindResource("GhostButton") as Style,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Padding = new Thickness(0),
            Margin = new Thickness(0, 2, 0, 2),
            Cursor = Cursors.Hand,
        };

        var row = new Grid { MinHeight = 42 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var title = string.IsNullOrWhiteSpace(group.displayName) ? "Group" : group.displayName;
        var avatar = new Avatar
        {
            Label = title,
            PictureUrl = group.pictureUrl,
            Size = 34,
            Margin = new Thickness(0, 0, 10, 0),
        };
        Grid.SetColumn(avatar, 0);
        row.Children.Add(avatar);

        var labels = new StackPanel
        {
            Orientation = Orientation.Vertical,
            VerticalAlignment = VerticalAlignment.Center,
        };
        labels.Children.Add(new TextBlock
        {
            Text = title,
            Foreground = ResourceBrush("TextPrimary"),
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        labels.Children.Add(new TextBlock
        {
            Text = $"{group.memberCount} people",
            Foreground = ResourceBrush("TextMuted"),
            FontSize = 12,
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        Grid.SetColumn(labels, 1);
        row.Children.Add(labels);

        var chevron = new TextBlock
        {
            Text = "\uE974",
            FontFamily = new FontFamily("Segoe MDL2 Assets"),
            Foreground = ResourceBrush("TextMuted"),
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(10, 0, 0, 0),
        };
        Grid.SetColumn(chevron, 2);
        row.Children.Add(chevron);

        button.Content = row;
        button.Click += (_, _) =>
        {
            var groupId = GroupIdFromChatId(group.chatId);
            if (string.IsNullOrEmpty(groupId)) return;
            window.Close();
            App.CurrentManager.Push(new Screen.GroupDetails(groupId));
        };
        return button;
    }

    private static FrameworkElement BuildDirectInfoButton(
        string title,
        Action action,
        bool destructive = false
    )
    {
        var button = new Button
        {
            Content = title,
            Padding = new Thickness(12, 8, 12, 8),
            HorizontalAlignment = HorizontalAlignment.Left,
            Margin = new Thickness(0, 0, 0, 8),
        };
        if (destructive)
        {
            button.Foreground = ResourceBrush("Danger");
        }
        button.Click += (_, _) => action();
        return button;
    }

    private void ShowBlockUserDialog(string chatId, string displayName)
    {
        ShowSafetyActionDialog(
            $"Block {displayName}?",
            "They will not be able to message you.",
            ("Block", true, () => App.CurrentManager.SetUserBlocked(chatId, true)),
            ("Block and report", true, () => ReportUser(chatId, displayName, block: true)),
            ("Delete chat", true, () =>
            {
                App.CurrentManager.DeleteChat(chatId);
                App.CurrentManager.NavigateBack();
            })
        );
    }

    private void ShowReportUserDialog(string chatId, string displayName)
    {
        ShowSafetyActionDialog(
            $"Report {displayName}?",
            "This prepares a report for support.",
            ("Report", true, () => ReportUser(chatId, displayName, block: false)),
            ("Block and report", true, () => ReportUser(chatId, displayName, block: true)),
            ("Delete chat", true, () =>
            {
                App.CurrentManager.DeleteChat(chatId);
                App.CurrentManager.NavigateBack();
            })
        );
    }

    private void ShowBlockAndReportUserDialog(string chatId, string displayName)
    {
        ShowSafetyActionDialog(
            $"Block and report {displayName}?",
            "This prepares a report for support and blocks this user.",
            ("Block and report", true, () => ReportUser(chatId, displayName, block: true)),
            ("Delete chat", true, () =>
            {
                App.CurrentManager.DeleteChat(chatId);
                App.CurrentManager.NavigateBack();
            })
        );
    }

    private void ShowSafetyActionDialog(
        string title,
        string message,
        params (string Title, bool Destructive, Action Action)[] actions
    )
    {
        var window = new Window
        {
            Title = title,
            Width = 360,
            SizeToContent = SizeToContent.Height,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            ShowInTaskbar = false,
            ResizeMode = ResizeMode.NoResize,
            Owner = Window.GetWindow(this),
            Background = ResourceBrush("Background"),
        };

        var stack = new StackPanel
        {
            Orientation = Orientation.Vertical,
            Margin = new Thickness(18),
        };
        stack.Children.Add(new TextBlock
        {
            Text = message,
            Foreground = ResourceBrush("TextMuted"),
            TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 12),
        });

        foreach (var action in actions)
        {
            stack.Children.Add(BuildDialogActionButton(window, action.Title, action.Destructive, action.Action));
        }
        stack.Children.Add(BuildDialogActionButton(window, "Cancel", false, () => { }));

        window.Content = stack;
        window.ShowDialog();
    }

    private static FrameworkElement BuildDialogActionButton(
        Window window,
        string title,
        bool destructive,
        Action action
    )
    {
        var button = new Button
        {
            Content = title,
            Padding = new Thickness(12, 8, 12, 8),
            HorizontalAlignment = HorizontalAlignment.Stretch,
            Margin = new Thickness(0, 0, 0, 8),
        };
        if (destructive)
        {
            button.Foreground = ResourceBrush("Danger");
        }
        button.Click += (_, _) =>
        {
            window.Close();
            action();
        };
        return button;
    }

    private static void ReportUser(string chatId, string displayName, bool block)
    {
        if (block)
        {
            App.CurrentManager.SetUserBlocked(chatId, true);
        }

        var userId = chatId;
        try
        {
            var npub = Native.PeerInputToNpub(chatId);
            if (!string.IsNullOrWhiteSpace(npub))
            {
                userId = npub;
            }
        }
        catch { }

        var body =
            $"Reported user: {displayName}\n" +
            $"User ID: {userId}\n" +
            "App: Iris Chat Windows\n\n" +
            "What happened:\n";
        var uri =
            $"mailto:{IrisSupportEmail}?subject={Uri.EscapeDataString("Iris Chat user report")}" +
            $"&body={Uri.EscapeDataString(body)}";
        if (!PlatformDocumentOpener.OpenUrl(uri))
        {
            PlatformClipboard.SetString(
                $"To: {IrisSupportEmail}\nSubject: Iris Chat user report\n\n{body}"
            );
            App.CurrentManager.ShowToast("Report details copied");
        }
    }

    private static string? GroupIdFromChatId(string chatId)
    {
        var trimmed = chatId.Trim();
        const string prefix = "group:";
        if (!trimmed.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)) return null;
        var groupId = trimmed[prefix.Length..].Trim();
        return string.IsNullOrEmpty(groupId) ? null : groupId;
    }

    private static Brush ResourceBrush(string key) =>
        (Brush)Application.Current.Resources[key];

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
