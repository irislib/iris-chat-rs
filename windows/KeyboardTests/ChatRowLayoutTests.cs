using System;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media.Imaging;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class ChatRowLayoutTests
{
    internal static void Run(Window window, ChatThreadSnapshot chat, string output)
    {
        foreach (var width in new[] { 280.0, 352.0, 520.0 })
        foreach (var scale in new[] { 1.0, 1.5 })
        {
            var row = new ChatRow { Width = width, Chat = chat };
            var preview = (TextBlock)row.FindName("PreviewText");
            preview.FontSize = 13 * scale;
            ((TextBlock)row.FindName("NameText")).FontSize = 15 * scale;
            var next = new TextBlock { Text = "Following chat", Height = 40 };
            var root = new StackPanel { Width = width, VerticalAlignment = VerticalAlignment.Top };
            root.Children.Add(row); root.Children.Add(next);
            window.Content = root; window.Show(); window.UpdateLayout();
            double? height = null, followingY = null;
            var samples = new[] { "", "Short preview", new string('W', 180), "One\nTwo\nThree", "Hello 👋 🎉" };
            foreach (var text in samples)
            foreach (var draft in new[] { false, true })
            foreach (var typing in new[] { false, true })
            {
                row.Chat = chat with { lastMessagePreview = text, draft = draft ? text : "", isTyping = typing,
                    unreadCount = draft ? 123UL : 0UL };
                window.UpdateLayout();
                var y = next.TranslatePoint(new Point(), root).Y;
                height ??= row.ActualHeight; followingY ??= y;
                if (row.ActualHeight <= 0 || Math.Abs(row.ActualHeight - height.Value) > 0.5 || Math.Abs(y - followingY.Value) > 0.5)
                    throw new Exception($"Chat preview moved a WPF row at width {width}, scale {scale}: {row.ActualHeight}/{y}");
            }
            if (width == 352 && scale == 1)
            {
                var bitmap = new RenderTargetBitmap((int)root.ActualWidth, (int)root.ActualHeight, 96, 96, System.Windows.Media.PixelFormats.Pbgra32);
                bitmap.Render(root);
                var png = new PngBitmapEncoder(); png.Frames.Add(BitmapFrame.Create(bitmap));
                using var stream = File.Create(Path.Combine(output, "windows-chat-row-preview-layout.png")); png.Save(stream);
            }
        }
        Console.WriteLine("PASS: WPF preview/typing/draft/newline/unread rows and following position stay stable at 3 widths and 2 font sizes");
    }
}
