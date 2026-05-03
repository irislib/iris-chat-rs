using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

public class MessageInfoWindow : Window
{
    private readonly ChatMessageSnapshot _message;

    public MessageInfoWindow(ChatMessageSnapshot message)
    {
        _message = message;
        Title = "Message info";
        Width = 460;
        Height = 600;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        ShowInTaskbar = false;
        ResizeMode = ResizeMode.CanResize;
        if (Application.Current?.Resources["Background"] is Brush bg)
        {
            Background = bg;
        }
        Content = BuildContent();
    }

    private FrameworkElement BuildContent()
    {
        var scroll = new ScrollViewer
        {
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            Padding = new Thickness(20, 18, 20, 18),
        };

        var stack = new StackPanel { Orientation = Orientation.Vertical };

        stack.Children.Add(BuildHeader());
        stack.Children.Add(BuildSection("Status", BuildStatusRows()));
        stack.Children.Add(BuildSection("People", BuildPeopleRows()));
        stack.Children.Add(BuildSection("IDs", BuildIdRows()));
        if (_message.attachments != null && _message.attachments.Length > 0)
        {
            stack.Children.Add(BuildSection("Attachments", BuildAttachmentRows()));
        }
        if ((_message.reactions != null && _message.reactions.Length > 0) ||
            (_message.reactors != null && _message.reactors.Length > 0))
        {
            stack.Children.Add(BuildSection("Reactions", BuildReactionRows()));
        }

        scroll.Content = stack;
        return scroll;
    }

    private FrameworkElement BuildHeader()
    {
        var grid = new Grid { Margin = new Thickness(0, 0, 0, 12) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var status = new StackPanel { Orientation = Orientation.Vertical, VerticalAlignment = VerticalAlignment.Center };
        status.Children.Add(new TextBlock
        {
            Text = DeliveryLabel(_message.delivery),
            FontSize = 18,
            FontWeight = FontWeights.SemiBold,
            Foreground = (Brush)Application.Current.Resources["TextPrimary"],
        });
        status.Children.Add(new TextBlock
        {
            Text = DirectionLabel(_message),
            FontSize = 12,
            Foreground = (Brush)Application.Current.Resources["TextMuted"],
        });
        Grid.SetColumn(status, 0);
        grid.Children.Add(status);

        var copy = new Button
        {
            Content = "Copy",
            Padding = new Thickness(12, 4, 12, 4),
            VerticalAlignment = VerticalAlignment.Center,
        };
        copy.Click += (_, _) =>
        {
            try { Clipboard.SetText(MessageInfoText(_message)); }
            catch { /* clipboard contention */ }
        };
        Grid.SetColumn(copy, 1);
        grid.Children.Add(copy);

        return grid;
    }

    private FrameworkElement BuildSection(string title, IList<UIElement> rows)
    {
        var border = new Border
        {
            Background = (Brush)Application.Current.Resources["Panel"],
            CornerRadius = new CornerRadius(12),
            Padding = new Thickness(16, 12, 16, 12),
            Margin = new Thickness(0, 0, 0, 10),
        };
        var stack = new StackPanel { Orientation = Orientation.Vertical };
        stack.Children.Add(new TextBlock
        {
            Text = title,
            FontWeight = FontWeights.SemiBold,
            Foreground = (Brush)Application.Current.Resources["TextPrimary"],
            Margin = new Thickness(0, 0, 0, 6),
        });
        foreach (var row in rows) stack.Children.Add(row);
        border.Child = stack;
        return border;
    }

    private List<UIElement> BuildStatusRows()
    {
        var rows = new List<UIElement>
        {
            ValueRow("Time", FormatDateTime(_message.createdAtSecs)),
        };
        if (_message.expiresAtSecs.HasValue)
        {
            rows.Add(ValueRow("Deletes", FormatDateTime(_message.expiresAtSecs.Value)));
        }
        rows.Add(ValueRow("Type", KindLabel(_message)));
        return rows;
    }

    private List<UIElement> BuildPeopleRows()
    {
        var rows = new List<UIElement>();
        if (_message.isOutgoing)
        {
            rows.Add(ValueRow("You", $"{DeliveryLabel(_message.delivery)} · {FormatDateTime(_message.createdAtSecs)}"));
        }
        else
        {
            rows.Add(ValueRow("From", _message.author));
            rows.Add(ValueRow("Status", DeliveryLabel(_message.delivery)));
        }
        return rows;
    }

    private List<UIElement> BuildIdRows()
    {
        var rows = new List<UIElement>
        {
            CopyRow("Message", _message.id),
        };
        if (!string.IsNullOrEmpty(_message.sourceEventId))
        {
            rows.Add(CopyRow("Received event", _message.sourceEventId!));
        }
        return rows;
    }

    private List<UIElement> BuildAttachmentRows()
    {
        var rows = new List<UIElement>();
        foreach (var att in _message.attachments!)
        {
            rows.Add(CopyRow(string.IsNullOrEmpty(att.filename) ? "File" : att.filename, att.htreeUrl));
        }
        return rows;
    }

    private List<UIElement> BuildReactionRows()
    {
        var rows = new List<UIElement>();
        if (_message.reactions != null)
        {
            foreach (var reaction in _message.reactions)
            {
                rows.Add(ValueRow(reaction.emoji, reaction.count.ToString()));
            }
        }
        if (_message.reactors != null)
        {
            foreach (var reactor in _message.reactors)
            {
                var value = string.IsNullOrEmpty(reactor.emoji) ? "Removed" : reactor.emoji;
                rows.Add(ValueRow(ShortIdentifier(reactor.author), value));
            }
        }
        return rows;
    }

    private UIElement ValueRow(string label, string value, bool monospace = false)
    {
        var grid = new Grid { Margin = new Thickness(0, 3, 0, 3) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(120) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var labelBlock = new TextBlock
        {
            Text = label,
            FontSize = 12,
            Foreground = (Brush)Application.Current.Resources["TextMuted"],
            VerticalAlignment = VerticalAlignment.Top,
        };
        Grid.SetColumn(labelBlock, 0);
        grid.Children.Add(labelBlock);
        var valueBlock = new TextBlock
        {
            Text = value,
            FontSize = 13,
            Foreground = (Brush)Application.Current.Resources["TextPrimary"],
            TextWrapping = TextWrapping.Wrap,
        };
        if (monospace)
        {
            valueBlock.FontFamily = new FontFamily("Consolas");
            valueBlock.FontSize = 12;
        }
        Grid.SetColumn(valueBlock, 1);
        grid.Children.Add(valueBlock);
        return grid;
    }

    private UIElement CopyRow(string label, string value)
    {
        var grid = new Grid { Margin = new Thickness(0, 3, 0, 3) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(120) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var labelBlock = new TextBlock
        {
            Text = label,
            FontSize = 12,
            Foreground = (Brush)Application.Current.Resources["TextMuted"],
            VerticalAlignment = VerticalAlignment.Top,
        };
        Grid.SetColumn(labelBlock, 0);
        grid.Children.Add(labelBlock);
        var valueBlock = new TextBlock
        {
            Text = ShortIdentifier(value),
            FontSize = 12,
            FontFamily = new FontFamily("Consolas"),
            Foreground = (Brush)Application.Current.Resources["TextPrimary"],
            TextWrapping = TextWrapping.Wrap,
            VerticalAlignment = VerticalAlignment.Center,
        };
        Grid.SetColumn(valueBlock, 1);
        grid.Children.Add(valueBlock);
        var copy = new Button
        {
            Content = "Copy",
            Padding = new Thickness(8, 2, 8, 2),
            FontSize = 11,
            VerticalAlignment = VerticalAlignment.Center,
        };
        copy.Click += (_, _) =>
        {
            try { Clipboard.SetText(value); }
            catch { /* clipboard contention */ }
        };
        Grid.SetColumn(copy, 2);
        grid.Children.Add(copy);
        return grid;
    }

    private static string DirectionLabel(ChatMessageSnapshot message)
    {
        if (message.kind == ChatMessageKind.System) return "System message";
        return message.isOutgoing ? "Sent message" : "Received message";
    }

    private static string KindLabel(ChatMessageSnapshot message) =>
        message.kind == ChatMessageKind.System ? "System" :
        (message.isOutgoing ? "Sent" : "Received");

    private static string DeliveryLabel(DeliveryState delivery) => delivery switch
    {
        DeliveryState.Queued => "Queued",
        DeliveryState.Pending => "Pending",
        DeliveryState.Sent => "Sent",
        DeliveryState.Received => "Received",
        DeliveryState.Seen => "Seen",
        DeliveryState.Failed => "Failed",
        _ => string.Empty,
    };

    private static string FormatDateTime(ulong secs)
    {
        try
        {
            return DateTimeOffset.FromUnixTimeSeconds((long)secs)
                .LocalDateTime
                .ToString("MMM d, yyyy · HH:mm");
        }
        catch
        {
            return secs.ToString();
        }
    }

    private static string ShortIdentifier(string value)
    {
        if (string.IsNullOrEmpty(value) || value.Length <= 16) return value ?? string.Empty;
        return $"{value[..8]}...{value[^8..]}";
    }

    private static string MessageInfoText(ChatMessageSnapshot message)
    {
        var sb = new StringBuilder();
        sb.AppendLine($"Message {message.id}");
        sb.AppendLine($"Time {FormatDateTime(message.createdAtSecs)}");
        sb.AppendLine($"Type {KindLabel(message)}");
        sb.AppendLine($"Status {DeliveryLabel(message.delivery)}");
        if (message.expiresAtSecs.HasValue)
        {
            sb.AppendLine($"Deletes {FormatDateTime(message.expiresAtSecs.Value)}");
        }
        if (!string.IsNullOrEmpty(message.sourceEventId))
        {
            sb.AppendLine($"Received as {ShortIdentifier(message.sourceEventId!)}");
        }
        if (message.attachments != null && message.attachments.Length > 0)
        {
            sb.AppendLine("Attachments");
            foreach (var att in message.attachments)
            {
                sb.AppendLine($"- {(string.IsNullOrEmpty(att.filename) ? "File" : att.filename)} {att.htreeUrl}");
            }
        }
        if (message.reactions != null && message.reactions.Length > 0)
        {
            sb.AppendLine("Reactions");
            foreach (var reaction in message.reactions)
            {
                sb.AppendLine($"- {reaction.emoji} {reaction.count}");
            }
        }
        return sb.ToString().TrimEnd();
    }
}
