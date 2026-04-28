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

public partial class NewGroupView : UserControl
{
    private readonly HashSet<string> _selectedOwners = new();

    public NewGroupView()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            UpdateBusy();
            RebuildKnownUsers();
            RebuildSelected();
            NameInput.Focus();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e)
    {
        UpdateBusy();
        if (e.PropertyName == nameof(App.CurrentManager.ChatList) || e.PropertyName == null)
        {
            RebuildKnownUsers();
        }
    }

    private void UpdateBusy() =>
        CreateButton.IsEnabled = !App.CurrentManager.Busy.creatingGroup;

    private string LocalOwnerHex => App.CurrentManager.Account?.publicKeyHex ?? string.Empty;

    private IEnumerable<ChatThreadSnapshot> KnownUsers =>
        App.CurrentManager.ChatList
            .Where(c => c.kind == ChatKind.Direct && c.chatId != LocalOwnerHex);

    private IEnumerable<ChatThreadSnapshot> FilteredKnownUsers
    {
        get
        {
            var query = (MemberSearchInput.Text ?? string.Empty).Trim();
            if (query.Length == 0) return KnownUsers;
            var lower = query.ToLowerInvariant();
            return KnownUsers.Where(c =>
                (c.displayName ?? string.Empty).ToLowerInvariant().Contains(lower)
                || (c.chatId ?? string.Empty).ToLowerInvariant().Contains(lower)
                || ((c.subtitle ?? string.Empty).ToLowerInvariant().Contains(lower)));
        }
    }

    private void OnSearchTextChanged(object sender, TextChangedEventArgs e) => RebuildKnownUsers();

    private void OnAddMemberClick(object sender, RoutedEventArgs e)
    {
        var input = (MemberSearchInput.Text ?? string.Empty).Trim();
        if (string.IsNullOrEmpty(input)) return;
        AddOwner(input);
    }

    private void AddOwner(string owner)
    {
        var trimmed = owner.Trim();
        if (string.IsNullOrEmpty(trimmed)) return;
        if (string.Equals(trimmed, LocalOwnerHex, StringComparison.OrdinalIgnoreCase)) return;
        if (_selectedOwners.Add(trimmed))
        {
            MemberSearchInput.Clear();
            RebuildSelected();
            RebuildKnownUsers();
        }
    }

    private void RemoveOwner(string owner)
    {
        if (_selectedOwners.Remove(owner))
        {
            RebuildSelected();
            RebuildKnownUsers();
        }
    }

    private void RebuildSelected()
    {
        SelectedMembersList.Items.Clear();
        foreach (var owner in _selectedOwners.OrderBy(s => s))
        {
            var chat = KnownUsers.FirstOrDefault(c =>
                string.Equals(c.chatId, owner, StringComparison.OrdinalIgnoreCase));
            var label = !string.IsNullOrWhiteSpace(chat?.displayName) ? chat!.displayName : "Iris user";
            SelectedMembersList.Items.Add(BuildSelectedRow(owner, label));
        }
    }

    private void RebuildKnownUsers()
    {
        KnownUsersList.Items.Clear();
        var rows = FilteredKnownUsers
            .Where(c => !_selectedOwners.Contains(c.chatId))
            .ToList();
        foreach (var chat in rows)
        {
            KnownUsersList.Items.Add(BuildKnownUserRow(chat));
        }
        var query = (MemberSearchInput.Text ?? string.Empty).Trim();
        var headerText = query.Length == 0 ? "Known users" : "Search results";
        KnownUsersHeader.Text = headerText;
        KnownUsersHeader.Visibility = rows.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    private FrameworkElement BuildKnownUserRow(ChatThreadSnapshot chat)
    {
        var grid = new Grid { Margin = new Thickness(0, 4, 0, 4) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var avatar = new IrisChat.Chrome.Avatar
        {
            Label = chat.displayName ?? string.Empty,
            Size = 36,
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
        addBtn.Click += (_, _) => AddOwner(chat.chatId);
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
        border.MouseLeftButtonUp += (_, _) => AddOwner(chat.chatId);
        return border;
    }

    private FrameworkElement BuildSelectedRow(string owner, string label)
    {
        var grid = new Grid { Margin = new Thickness(0, 2, 0, 2) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var text = new TextBlock
        {
            Text = label,
            VerticalAlignment = VerticalAlignment.Center,
            Foreground = (Brush)Application.Current.Resources["TextPrimary"],
        };
        Grid.SetColumn(text, 0);

        var removeBtn = new Button
        {
            Style = (Style)FindResource("CompactSecondaryButton"),
            Content = new TextBlock
            {
                Text = "Remove",
                Foreground = (Brush)Application.Current.Resources["Danger"],
            },
        };
        removeBtn.Click += (_, _) => RemoveOwner(owner);
        Grid.SetColumn(removeBtn, 1);

        grid.Children.Add(text);
        grid.Children.Add(removeBtn);

        return new Border
        {
            Background = (Brush)Application.Current.Resources["Panel"],
            CornerRadius = (CornerRadius)Application.Current.Resources["SectionRadius"],
            Padding = new Thickness(10, 6, 10, 6),
            Child = grid,
        };
    }

    private void OnCreate(object sender, RoutedEventArgs e)
    {
        var name = NameInput.Text?.Trim();
        if (string.IsNullOrEmpty(name)) return;
        if (_selectedOwners.Count == 0) return;
        App.CurrentManager.CreateGroup(name, _selectedOwners.ToArray());
    }
}
