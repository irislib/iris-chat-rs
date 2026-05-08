using System;
using System.Collections.Concurrent;
using System.IO;
using System.Net.Http;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace IrisChat.Chrome;

public partial class Avatar : UserControl
{
    private static readonly ConcurrentDictionary<string, ImageSource> ImageCache = new();
    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(15) };
    private const int AvatarDecodePixelWidth = 160;
    private string? _loadingKey;

    public static readonly DependencyProperty LabelProperty =
        DependencyProperty.Register(nameof(Label), typeof(string), typeof(Avatar),
            new PropertyMetadata(string.Empty, OnLabelChanged));

    public static readonly DependencyProperty PictureUrlProperty =
        DependencyProperty.Register(nameof(PictureUrl), typeof(string), typeof(Avatar),
            new PropertyMetadata(null, OnPictureUrlChanged));

    public static readonly DependencyProperty SizeProperty =
        DependencyProperty.Register(nameof(Size), typeof(double), typeof(Avatar),
            new PropertyMetadata(44.0, OnSizeChanged));

    public string Label
    {
        get => (string)GetValue(LabelProperty);
        set => SetValue(LabelProperty, value);
    }

    public string? PictureUrl
    {
        get => (string?)GetValue(PictureUrlProperty);
        set => SetValue(PictureUrlProperty, value);
    }

    public double Size
    {
        get => (double)GetValue(SizeProperty);
        set => SetValue(SizeProperty, value);
    }

    public Avatar()
    {
        InitializeComponent();
        UpdateLabel();
        UpdateSize();
    }

    private static void OnLabelChanged(DependencyObject d, DependencyPropertyChangedEventArgs e) =>
        ((Avatar)d).UpdateLabel();

    private static void OnSizeChanged(DependencyObject d, DependencyPropertyChangedEventArgs e) =>
        ((Avatar)d).UpdateSize();

    private static void OnPictureUrlChanged(DependencyObject d, DependencyPropertyChangedEventArgs e) =>
        ((Avatar)d).UpdateImage();

    private void UpdateSize()
    {
        Width = Size;
        Height = Size;
        BackgroundBorder.CornerRadius = new CornerRadius(Size / 2);
        ImageHost.CornerRadius = new CornerRadius(Size / 2);
        Initials.FontSize = Size * 0.36;
    }

    private void UpdateLabel()
    {
        var label = Label ?? string.Empty;
        Initials.Text = ComputeInitials(label);
        BackgroundBorder.Background = new SolidColorBrush(ColorFor(label));
    }

    private async void UpdateImage()
    {
        var url = PictureUrl?.Trim();
        if (string.IsNullOrEmpty(url))
        {
            _loadingKey = null;
            ImageHost.Visibility = Visibility.Collapsed;
            return;
        }

        var key = CacheKey(url);
        _loadingKey = key;
        if (ImageCache.TryGetValue(key, out var cached))
        {
            ImageBrush.ImageSource = cached;
            ImageHost.Visibility = Visibility.Visible;
            return;
        }

        ImageHost.Visibility = Visibility.Collapsed;

        try
        {
            var data = await LoadImageBytesAsync(url);

            if (_loadingKey != key) return;

            if (data == null || data.Length == 0)
            {
                ImageHost.Visibility = Visibility.Collapsed;
                return;
            }

            var bmp = await Task.Run(() => DecodeAvatarImage(data));
            if (bmp == null)
            {
                if (_loadingKey == key)
                {
                    ImageHost.Visibility = Visibility.Collapsed;
                }
                return;
            }
            if (_loadingKey != key) return;

            ImageCache[key] = bmp;
            ImageBrush.ImageSource = bmp;
            ImageHost.Visibility = Visibility.Visible;
        }
        catch
        {
            if (_loadingKey == key)
            {
                ImageHost.Visibility = Visibility.Collapsed;
            }
        }
    }

    private static async Task<byte[]?> LoadImageBytesAsync(string url)
    {
        if (TryParseNhash(url, out var nhash))
        {
            return Application.Current is App app && app.Manager != null
                ? await app.Manager.ResolveProfilePictureAsync(nhash)
                : null;
        }
        if (url.StartsWith("http://", StringComparison.OrdinalIgnoreCase) ||
            url.StartsWith("https://", StringComparison.OrdinalIgnoreCase))
        {
            return await Http.GetByteArrayAsync(url);
        }
        return File.Exists(url) ? await File.ReadAllBytesAsync(url) : null;
    }

    private static BitmapImage? DecodeAvatarImage(byte[] data)
    {
        try
        {
            using var ms = new MemoryStream(data);
            var bmp = new BitmapImage();
            bmp.BeginInit();
            bmp.CacheOption = BitmapCacheOption.OnLoad;
            bmp.DecodePixelWidth = AvatarDecodePixelWidth;
            bmp.StreamSource = ms;
            bmp.EndInit();
            bmp.Freeze();
            return bmp;
        }
        catch
        {
            return null;
        }
    }

    private static string CacheKey(string url) =>
        TryParseNhash(url, out var nhash) ? $"htree:{nhash}" : url;

    private static bool TryParseNhash(string url, out string nhash)
    {
        var trimmed = url.Trim();
        if (trimmed.StartsWith("htree://", StringComparison.OrdinalIgnoreCase))
        {
            nhash = trimmed.Substring("htree://".Length).Split('/')[0];
            return !string.IsNullOrWhiteSpace(nhash);
        }
        if (trimmed.StartsWith("nhash://", StringComparison.OrdinalIgnoreCase))
        {
            nhash = trimmed.Substring("nhash://".Length).Split('/')[0];
            return !string.IsNullOrWhiteSpace(nhash);
        }
        nhash = string.Empty;
        return false;
    }

    private static string ComputeInitials(string label)
    {
        var trimmed = (label ?? string.Empty).Trim();
        if (string.IsNullOrEmpty(trimmed)) return "?";
        var parts = trimmed.Split(new[] { ' ', '\t' }, StringSplitOptions.RemoveEmptyEntries);
        if (parts.Length == 0) return char.ToUpperInvariant(trimmed[0]).ToString();
        if (parts.Length == 1) return char.ToUpperInvariant(parts[0][0]).ToString();
        return $"{char.ToUpperInvariant(parts[0][0])}{char.ToUpperInvariant(parts[^1][0])}";
    }

    private static Color ColorFor(string label)
    {
        unchecked
        {
            uint hash = 2166136261;
            foreach (var c in label ?? string.Empty)
            {
                hash ^= c;
                hash *= 16777619;
            }
            byte r = (byte)((hash >> 16) & 0xFF);
            byte g = (byte)((hash >> 8) & 0xFF);
            byte b = (byte)(hash & 0xFF);
            // Brighten so colors stay vivid on the dark background.
            r = (byte)(80 + r % 156);
            g = (byte)(80 + g % 156);
            b = (byte)(80 + b % 156);
            return Color.FromRgb(r, g, b);
        }
    }
}
