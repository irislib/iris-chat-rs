using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using IrisChat.Bindings;

namespace IrisChat.Views;

public partial class GroupDetailsView : UserControl
{
    public string? GroupId { get; set; }

    public GroupDetailsView()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            Refresh();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => Refresh();

    private void Refresh()
    {
        var details = App.CurrentManager.GroupDetails;
        if (details == null || (GroupId != null && details.groupId != GroupId)) return;

        GroupAvatar.Label = details.name;
        GroupAvatar.PictureUrl = details.pictureUrl;
        if (!GroupNameInput.IsKeyboardFocused)
            GroupNameInput.Text = details.name;
        GroupSubtitle.Text = $"{details.members.Length} members · created by {details.createdByDisplayName}";
        SaveNameButton.IsEnabled = details.canManage;
        AddMemberInput.IsEnabled = details.canManage;
        MuteChatText.Text = details.isMuted ? "Unmute chat" : "Mute chat";

        MembersList.Items.Clear();
        foreach (var m in details.members)
        {
            MembersList.Items.Add(BuildMember(details, m));
        }

        RebuildKnownUsers();
    }

    private void OnAddMemberTextChanged(object sender, TextChangedEventArgs e) => RebuildKnownUsers();

    private void RebuildKnownUsers()
    {
        var details = App.CurrentManager.GroupDetails;
        if (details == null || !details.canManage)
        {
            KnownUsersList.Items.Clear();
            KnownUsersHeader.Visibility = Visibility.Collapsed;
            return;
        }

        var localOwnerHex = App.CurrentManager.Account?.publicKeyHex ?? string.Empty;
        var memberHexes = new HashSet<string>(details.members.Select(m => m.ownerPubkeyHex));
        var query = (AddMemberInput.Text ?? string.Empty).Trim().ToLowerInvariant();

        var candidates = App.CurrentManager.ChatList
            .Where(c => c.kind == ChatKind.Direct
                && c.chatId != localOwnerHex
                && !memberHexes.Contains(c.chatId))
            .Where(c => query.Length == 0
                || (c.displayName ?? string.Empty).ToLowerInvariant().Contains(query)
                || (c.chatId ?? string.Empty).ToLowerInvariant().Contains(query)
                || (c.subtitle ?? string.Empty).ToLowerInvariant().Contains(query))
            .ToList();

        KnownUsersList.Items.Clear();
        foreach (var chat in candidates)
        {
            KnownUsersList.Items.Add(BuildKnownUserRow(details.groupId, chat));
        }
        KnownUsersHeader.Text = query.Length == 0 ? "Known users" : "Search results";
        KnownUsersHeader.Visibility = candidates.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    private FrameworkElement BuildKnownUserRow(string groupId, ChatThreadSnapshot chat)
    {
        var grid = new Grid { Margin = new Thickness(0, 4, 0, 4) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var avatar = new IrisChat.Chrome.Avatar
        {
            Label = chat.displayName ?? string.Empty,
            Size = 32,
            VerticalAlignment = VerticalAlignment.Center,
        };
        Grid.SetColumn(avatar, 0);

        var info = new StackPanel
        {
            Margin = new Thickness(10, 0, 0, 0),
            VerticalAlignment = VerticalAlignment.Center,
        };
        info.Children.Add(new TextBlock
        {
            Text = string.IsNullOrWhiteSpace(chat.displayName) ? "Iris user" : chat.displayName,
            FontWeight = FontWeights.SemiBold,
            Foreground = (Brush)Application.Current.Resources["TextPrimary"],
        });
        if (!string.IsNullOrWhiteSpace(chat.subtitle))
        {
            info.Children.Add(new TextBlock
            {
                Text = chat.subtitle,
                FontSize = 12,
                Foreground = (Brush)Application.Current.Resources["TextMuted"],
                TextTrimming = TextTrimming.CharacterEllipsis,
            });
        }
        Grid.SetColumn(info, 1);

        var addBtn = new Button
        {
            Style = (Style)FindResource("CompactSecondaryButton"),
            Content = new TextBlock { Text = "Add" },
            VerticalAlignment = VerticalAlignment.Center,
        };
        addBtn.Click += (_, _) => OnKnownUserAdd(groupId, chat.chatId);
        Grid.SetColumn(addBtn, 2);

        grid.Children.Add(avatar);
        grid.Children.Add(info);
        grid.Children.Add(addBtn);

        var border = new Border
        {
            Background = Brushes.Transparent,
            Padding = new Thickness(6),
            Cursor = Cursors.Hand,
            Child = grid,
        };
        border.MouseLeftButtonUp += (_, _) => OnKnownUserAdd(groupId, chat.chatId);
        return border;
    }

    private void OnKnownUserAdd(string groupId, string ownerHex)
    {
        if (string.IsNullOrEmpty(ownerHex)) return;
        App.CurrentManager.AddGroupMembers(groupId, new[] { ownerHex });
        AddMemberInput.Clear();
    }

    private FrameworkElement BuildMember(GroupDetailsSnapshot details, GroupMemberSnapshot m)
    {
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var info = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        info.Children.Add(new TextBlock
        {
            Text = m.displayName + (m.isLocalOwner ? " (you)" : string.Empty),
            Foreground = (Brush)Application.Current.Resources["TextPrimary"],
            FontWeight = FontWeights.SemiBold,
        });
        info.Children.Add(new TextBlock
        {
            Text = m.isCreator ? "creator" : (m.isAdmin ? "admin" : "member"),
            Foreground = (Brush)Application.Current.Resources["TextMuted"],
            FontSize = 12,
            Margin = new Thickness(0, 2, 0, 0),
        });
        Grid.SetColumn(info, 0);

        var actions = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            VerticalAlignment = VerticalAlignment.Center,
        };

        if (details.canManage && !m.isLocalOwner)
        {
            var toggleAdmin = new Button
            {
                Style = (Style)FindResource("CompactSecondaryButton"),
                Content = new TextBlock { Text = m.isAdmin ? "Remove admin" : "Make admin" },
                Margin = new Thickness(0, 0, 6, 0),
            };
            toggleAdmin.Click += (_, _) =>
                App.CurrentManager.SetGroupAdmin(details.groupId, m.ownerPubkeyHex, !m.isAdmin);
            actions.Children.Add(toggleAdmin);

            var remove = new Button
            {
                Style = (Style)FindResource("CompactSecondaryButton"),
                Content = new TextBlock
                {
                    Text = "Remove",
                    Foreground = (Brush)Application.Current.Resources["Danger"],
                },
            };
            remove.Click += (_, _) =>
                App.CurrentManager.RemoveGroupMember(details.groupId, m.ownerPubkeyHex);
            actions.Children.Add(remove);
        }

        Grid.SetColumn(actions, 1);

        grid.Children.Add(info);
        grid.Children.Add(actions);

        return new Border
        {
            Background = Brushes.Transparent,
            Padding = new Thickness(8, 8, 8, 8),
            BorderBrush = (Brush)Application.Current.Resources["Border"],
            BorderThickness = new Thickness(0, 0, 0, 1),
            Child = grid,
        };
    }

    private void OnSaveName(object sender, RoutedEventArgs e)
    {
        var details = App.CurrentManager.GroupDetails;
        if (details == null) return;
        var newName = GroupNameInput.Text?.Trim();
        if (string.IsNullOrEmpty(newName) || newName == details.name) return;
        App.CurrentManager.UpdateGroupName(details.groupId, newName!);
    }

    private void OnPickPicture(object sender, RoutedEventArgs e)
    {
        var details = App.CurrentManager.GroupDetails;
        if (details == null) return;
        var file = PlatformFilePicker.PickImage("Choose group picture");
        if (string.IsNullOrEmpty(file)) return;
        App.CurrentManager.UpdateGroupPicture(details.groupId, file!);
    }

    private void OnAddMember(object sender, RoutedEventArgs e)
    {
        var details = App.CurrentManager.GroupDetails;
        if (details == null) return;
        var input = AddMemberInput.Text?.Trim();
        if (string.IsNullOrEmpty(input)) return;
        App.CurrentManager.AddGroupMembers(details.groupId, new[] { input });
        AddMemberInput.Clear();
    }

    private void OnDeleteChat(object sender, RoutedEventArgs e)
    {
        var details = App.CurrentManager.GroupDetails;
        if (details == null) return;
        App.CurrentManager.DeleteChat($"group:{details.groupId}");
    }

    private void OnToggleMute(object sender, RoutedEventArgs e)
    {
        var details = App.CurrentManager.GroupDetails;
        if (details == null) return;
        App.CurrentManager.SetChatMuted($"group:{details.groupId}", !details.isMuted);
    }
}
