using System;
using System.Text;
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

public partial class ChatView
{
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
            Text = "Nickname and note",
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

        if (!string.IsNullOrWhiteSpace(chat.contactNote))
        {
            stack.Children.Add(new TextBlock
            {
                Text = chat.contactNote,
                TextWrapping = TextWrapping.Wrap,
                Foreground = ResourceBrush("TextPrimary"),
                Margin = new Thickness(0, 12, 0, 0),
            });
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
            Title = "Nickname and note",
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
        stack.Children.Add(new TextBlock { Text = "Only you can see this.", Foreground = ResourceBrush("TextMuted"), Margin = new Thickness(0, 0, 0, 12) });
        stack.Children.Add(new TextBlock { Text = "Nickname", Foreground = ResourceBrush("TextPrimary") });
        var input = new TextBox
        {
            Text = chat.nickname ?? string.Empty,
            MinWidth = 260,
            Margin = new Thickness(0, 0, 0, 14),
        };
        stack.Children.Add(input);
        stack.Children.Add(new TextBlock { Text = "Note", Foreground = ResourceBrush("TextPrimary") });
        var note = new TextBox
        {
            Text = chat.contactNote ?? string.Empty,
            AcceptsReturn = true,
            TextWrapping = TextWrapping.Wrap,
            MinHeight = 90,
            MaxHeight = 180,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            Margin = new Thickness(0, 0, 0, 8),
        };
        stack.Children.Add(note);
        var count = new TextBlock { Foreground = ResourceBrush("TextMuted"), Margin = new Thickness(0, 0, 0, 12) };
        stack.Children.Add(count);

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
            App.CurrentManager.SetContactDetails(chat.chatId, input.Text, note.Text);
            dialog.Close();
        };
        void Validate()
        {
            var nickname = Regex.Replace(input.Text.Trim(), @"\s+", " ");
            var noteText = note.Text.Replace("\r\n", "\n").Replace('\r', '\n').Trim();
            var noteLength = noteText.EnumerateRunes().Count();
            save.IsEnabled = nickname.EnumerateRunes().Count() <= 80 && noteLength <= 240
                && (nickname != (chat.nickname ?? "") || noteText != (chat.contactNote ?? ""));
            count.Text = nickname.EnumerateRunes().Count() > 80 ? "Use up to 80 characters for a nickname." : noteLength >= 140 ? $"{noteLength}/240" : "";
        }
        input.TextChanged += (_, _) => Validate();
        note.TextChanged += (_, _) => Validate();
        Validate();
        actions.Children.Add(save);
        var cancel = new Button { Content = "Cancel", IsCancel = true, Padding = new Thickness(12, 7, 12, 7), Margin = new Thickness(0, 0, 8, 0) };
        cancel.Click += (_, _) => dialog.Close();
        actions.Children.Add(cancel);

        if (!string.IsNullOrWhiteSpace(chat.nickname) || !string.IsNullOrWhiteSpace(chat.contactNote))
        {
            var remove = new Button
            {
                Content = "Remove",
                Padding = new Thickness(12, 7, 12, 7),
            };
            remove.Click += (_, _) =>
            {
                App.CurrentManager.SetContactDetails(chat.chatId, string.Empty, string.Empty);
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

}
