using System;
using System.IO;
using System.Collections.Generic;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace IrisChat.Chrome;

public sealed class ImageViewerWindow : Window
{
    private static readonly List<ImageViewerWindow> OpenViewers = new();

    public static void CloseAll()
    {
        foreach (var viewer in OpenViewers.ToArray()) viewer.Close();
    }

    public ImageViewerWindow(BitmapSource bitmap, string filename, Action? openFile = null, Action<string>? showError = null)
    {
        Loaded += (_, _) => OpenViewers.Add(this);
        Closed += (_, _) => OpenViewers.Remove(this);
        Title = filename;
        Width = 900; Height = 700; MinWidth = 320; MinHeight = 240;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        Background = Brushes.Black;
        var image = new Image { Source = bitmap, Stretch = Stretch.Uniform, Margin = new Thickness(12) };
        var menu = new ContextMenu();
        var copy = new MenuItem { Header = "Copy image", InputGestureText = "Ctrl+C" };
        void Copy() { try { Clipboard.SetImage(bitmap); } catch { showError?.Invoke("Couldn’t copy image"); } }
        copy.Click += (_, _) => Copy();
        menu.Items.Add(copy);
        if (openFile != null)
        {
            var open = new MenuItem { Header = "Open file" };
            open.Click += (_, _) => openFile();
            menu.Items.Add(open);
        }
        image.ContextMenu = menu;
        Content = image;
        KeyDown += (_, e) => {
            if (e.Key == Key.Escape) { Close(); e.Handled = true; }
            else if (e.Key == Key.C && Keyboard.Modifiers.HasFlag(ModifierKeys.Control)) { Copy(); e.Handled = true; }
        };
    }

    public static BitmapSource? Decode(byte[] data)
    {
        try
        {
            using var stream = new MemoryStream(data);
            var image = new BitmapImage();
            image.BeginInit(); image.CacheOption = BitmapCacheOption.OnLoad; image.StreamSource = stream; image.EndInit();
            image.Freeze();
            return image;
        }
        catch { return null; }
    }
}
