using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Net.Http;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

public partial class Avatar : UserControl
{
    private static readonly ConcurrentDictionary<string, ImageSource> ImageCache = new();
    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(15) };
    private static readonly HttpClient ProxyHttp = new(new HttpClientHandler { AllowAutoRedirect = false })
        { Timeout = TimeSpan.FromSeconds(15) };
    private const int AvatarDecodePixelWidth = 160;
    private string? _loadingKey;
    private int _loadVersion;
    private AppManager? _preferencesManager;

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
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        _preferencesManager = Application.Current is App app ? app.Manager : null;
        if (_preferencesManager != null)
            _preferencesManager.PropertyChanged += OnPreferencesChanged;
        UpdateImage();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        if (_preferencesManager != null)
            _preferencesManager.PropertyChanged -= OnPreferencesChanged;
        _preferencesManager = null;
        _loadingKey = null;
        _loadVersion++;
    }

    private void OnPreferencesChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName == nameof(AppManager.Preferences)) UpdateImage();
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
        var urls = string.IsNullOrEmpty(url) ? Array.Empty<string>() : ImageUrls(url);
        var preferences = Application.Current is App app ? app.Manager?.Preferences : null;
        var allowOriginalRedirects = preferences != null &&
            (!preferences.imageProxyEnabled || preferences.imageProxyFallbackEnabled);
        var key = $"{allowOriginalRedirects}\n{string.Join("\n", urls)}";
        if (_loadingKey == key) return;
        _loadingKey = key;
        var version = ++_loadVersion;
        ImageHost.Visibility = Visibility.Collapsed;

        // Cache by the URL actually loaded. A cached original must never skip
        // the proxy when the user's current settings require it.
        foreach (var candidate in urls)
        {
            if (_loadVersion != version) return;
            var cacheKey = CacheKey(candidate);
            if (ImageCache.TryGetValue(cacheKey, out var cached))
            {
                ImageBrush.ImageSource = cached;
                ImageHost.Visibility = Visibility.Visible;
                return;
            }
            try
            {
                var data = await LoadImageBytesAsync(candidate, candidate != url || !allowOriginalRedirects);
                if (_loadVersion != version) return;
                if (data == null || data.Length == 0) continue;
                var bmp = await Task.Run(() => DecodeAvatarImage(data));
                if (_loadVersion != version) return;
                if (bmp == null) continue;
                ImageCache[cacheKey] = bmp;
                ImageBrush.ImageSource = bmp;
                ImageHost.Visibility = Visibility.Visible;
                return;
            }
            catch
            {
                // Only the shared policy can add an original URL to retry.
            }
        }
    }

    private static IReadOnlyList<string> ImageUrls(string url)
    {
        if (!url.StartsWith("http://", StringComparison.OrdinalIgnoreCase) &&
            !url.StartsWith("https://", StringComparison.OrdinalIgnoreCase))
            return new[] { url };
        return Application.Current is App app && app.Manager != null
            ? Native.ImageLoadUrls(url, app.Manager.Preferences,
                AvatarDecodePixelWidth, AvatarDecodePixelWidth, true)
            : Array.Empty<string>();
    }

    private static async Task<byte[]?> LoadImageBytesAsync(string url, bool isProxy)
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
            return isProxy ? await LoadProxyImageBytesAsync(url) : await Http.GetByteArrayAsync(url);
        }
        return File.Exists(url) ? await File.ReadAllBytesAsync(url) : null;
    }

    private static async Task<byte[]?> LoadProxyImageBytesAsync(string url)
    {
        var origin = new Uri(url);
        var current = origin;
        for (var redirects = 0; redirects < 10; redirects++)
        {
            using var response = await ProxyHttp.GetAsync(current);
            var status = (int)response.StatusCode;
            if (status is 301 or 302 or 303 or 307 or 308)
            {
                var location = response.Headers.Location;
                if (location == null) return null;
                var next = location.IsAbsoluteUri ? location : new Uri(current, location);
                if (next.Scheme != origin.Scheme || next.IdnHost != origin.IdnHost || next.Port != origin.Port)
                    return null;
                current = next;
                continue;
            }
            response.EnsureSuccessStatusCode();
            return await response.Content.ReadAsByteArrayAsync();
        }
        return null;
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
