using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

public partial class MessageBubble : UserControl
{
    private ChatMessageSnapshot? _message;

    public MessageBubble()
    {
        InitializeComponent();
    }

    public void Bind(ChatMessageSnapshot message, bool showAuthor = false, string? authorLabel = null)
    {
        _message = message;

        if (showAuthor && !string.IsNullOrEmpty(authorLabel))
        {
            AuthorText.Text = authorLabel;
            AuthorText.Visibility = Visibility.Visible;
        }
        else
        {
            AuthorText.Visibility = Visibility.Collapsed;
        }

        BodyText.Text = message.body ?? string.Empty;
        BodyText.Visibility = string.IsNullOrEmpty(message.body) ? Visibility.Collapsed : Visibility.Visible;

        if (message.kind == ChatMessageKind.System)
        {
            Bubble.Background = (Brush)FindResource("Panel");
            Bubble.HorizontalAlignment = HorizontalAlignment.Center;
            BodyText.Foreground = (Brush)FindResource("TextMuted");
            BodyText.HorizontalAlignment = HorizontalAlignment.Center;
            DeliveryText.Visibility = Visibility.Collapsed;
        }
        else if (message.isOutgoing)
        {
            Bubble.Background = (Brush)FindResource("BubbleMine");
            Bubble.HorizontalAlignment = HorizontalAlignment.Right;
            BodyText.Foreground = Brushes.White;
            DeliveryText.Visibility = Visibility.Visible;
        }
        else
        {
            Bubble.Background = (Brush)FindResource("BubbleTheirs");
            Bubble.HorizontalAlignment = HorizontalAlignment.Left;
            BodyText.Foreground = Brushes.White;
            DeliveryText.Visibility = Visibility.Collapsed;
        }

        TimeText.Text = DateTimeOffset.FromUnixTimeSeconds((long)message.createdAtSecs)
            .LocalDateTime.ToString("HH:mm");
        DeliveryText.Text = message.delivery switch
        {
            DeliveryState.Queued => "queued",
            DeliveryState.Pending => "sending",
            DeliveryState.Sent => "sent",
            DeliveryState.Received => "received",
            DeliveryState.Seen => "seen",
            DeliveryState.Failed => "failed",
            _ => string.Empty,
        };

        if (message.attachments != null && message.attachments.Length > 0)
        {
            AttachmentsList.ItemsSource = null;
            var panel = new StackPanel();
            foreach (var att in message.attachments)
            {
                panel.Children.Add(BuildAttachmentRow(att));
            }
            AttachmentsList.ItemsSource = panel.Children;
            AttachmentsList.Visibility = Visibility.Visible;
        }
        else
        {
            AttachmentsList.Visibility = Visibility.Collapsed;
        }
    }

    private FrameworkElement BuildAttachmentRow(MessageAttachmentSnapshot att)
    {
        if (att.isImage)
        {
            var image = new System.Windows.Controls.Image
            {
                Width = 320,
                MaxHeight = 280,
                Stretch = Stretch.Uniform,
                Margin = new Thickness(0, 4, 0, 0),
            };
            _ = LoadImageAsync(att, image);

            var btn = new Button
            {
                Style = (Style)FindResource("GhostButton"),
                Padding = new Thickness(0),
                Background = Brushes.Transparent,
                BorderBrush = Brushes.Transparent,
                Content = image,
                Margin = new Thickness(0, 0, 0, 4),
            };
            btn.Click += (_, _) => OpenAttachment(att);
            return btn;
        }

        var fileBtn = new Button
        {
            Style = (Style)FindResource("SecondaryButton"),
            Margin = new Thickness(0, 4, 0, 0),
            HorizontalContentAlignment = HorizontalAlignment.Left,
            Content = new StackPanel
            {
                Orientation = Orientation.Horizontal,
                Children =
                {
                    new TextBlock { Text = "📎 ", FontSize = 14, VerticalAlignment = VerticalAlignment.Center },
                    new TextBlock
                    {
                        Text = string.IsNullOrEmpty(att.filename) ? "Attachment" : att.filename,
                        Foreground = (Brush)FindResource("TextPrimary"),
                        FontSize = 13,
                        VerticalAlignment = VerticalAlignment.Center,
                    },
                },
            },
        };
        fileBtn.Click += (_, _) => OpenAttachment(att);
        return fileBtn;
    }

    private static async Task LoadImageAsync(MessageAttachmentSnapshot att, System.Windows.Controls.Image control)
    {
        if (Application.Current is not App app || app.Manager == null) return;
        var data = await app.Manager.DownloadAttachmentAsync(att);
        if (data == null) return;
        try
        {
            using var ms = new System.IO.MemoryStream(data);
            var bmp = new BitmapImage();
            bmp.BeginInit();
            bmp.CacheOption = BitmapCacheOption.OnLoad;
            bmp.StreamSource = ms;
            bmp.EndInit();
            bmp.Freeze();
            control.Source = bmp;
        }
        catch
        {
            // ignore decode errors
        }
    }

    private static void OpenAttachment(MessageAttachmentSnapshot att)
    {
        if (Application.Current is App app && app.Manager != null)
        {
            _ = app.Manager.OpenAttachmentAsync(att);
        }
    }

    private void OnCopyText(object sender, RoutedEventArgs e)
    {
        if (_message == null) return;
        var pieces = new List<string>();
        if (!string.IsNullOrEmpty(_message.body)) pieces.Add(_message.body);
        if (_message.attachments != null)
        {
            foreach (var att in _message.attachments)
            {
                pieces.Add(att.htreeUrl);
            }
        }
        if (pieces.Count == 0) return;
        try { Clipboard.SetText(string.Join("\n", pieces)); }
        catch { /* clipboard contention */ }
    }

    private void OnShowInfo(object sender, RoutedEventArgs e)
    {
        if (_message == null) return;
        var owner = Window.GetWindow(this);
        var window = new MessageInfoWindow(_message)
        {
            Owner = owner,
        };
        window.ShowDialog();
    }
}
