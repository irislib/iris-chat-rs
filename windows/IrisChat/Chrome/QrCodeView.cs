using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

/// Renders a QR code by asking the Rust core for the module matrix and
/// painting it as a black-on-white WriteableBitmap. Crisp pixels at any size.
public sealed class QrCodeView : Border
{
    public static readonly DependencyProperty TextProperty =
        DependencyProperty.Register(nameof(Text), typeof(string), typeof(QrCodeView),
            new PropertyMetadata(null, (d, _) => ((QrCodeView)d).Render()));

    // Each module is rendered as a small block of pixels; we always render at
    // a fixed 8 px/module and then let WPF's Stretch=Uniform scale to fit.
    private const int ModuleSize = 8;

    public string? Text
    {
        get => (string?)GetValue(TextProperty);
        set => SetValue(TextProperty, value);
    }

    private readonly Image _image;

    public QrCodeView()
    {
        Background = Brushes.White;
        CornerRadius = new CornerRadius(14);
        Padding = new Thickness(12);
        SnapsToDevicePixels = true;
        UseLayoutRounding = true;
        // Box ourselves into a sensible square so the contained image scales
        // uniformly. The Border's MaxWidth caps how big the QR can grow; the
        // Image fills the available square via Stretch=Uniform so any QR
        // version scales down to fit instead of overflowing.
        MaxWidth = 320;
        HorizontalAlignment = HorizontalAlignment.Center;
        _image = new Image
        {
            Stretch = Stretch.Uniform,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            VerticalAlignment = VerticalAlignment.Stretch,
        };
        RenderOptions.SetBitmapScalingMode(_image, BitmapScalingMode.NearestNeighbor);
        RenderOptions.SetEdgeMode(_image, EdgeMode.Aliased);
        Child = _image;
    }

    private void Render()
    {
        var text = Text;
        if (string.IsNullOrWhiteSpace(text))
        {
            _image.Source = null;
            return;
        }

        QrCodeMatrix? matrix = null;
        try { matrix = Native.EncodeTextQr(text); } catch { }
        if (matrix == null || matrix.size == 0 || string.IsNullOrEmpty(matrix.modules))
        {
            _image.Source = null;
            return;
        }

        var size = (int)matrix.size;
        var module = ModuleSize;
        var pixelSize = size * module;

        var bmp = new WriteableBitmap(pixelSize, pixelSize, 96, 96, PixelFormats.Bgra32, null);
        var stride = pixelSize * 4;
        var buffer = new byte[stride * pixelSize];

        // Initialize white.
        for (var i = 0; i < buffer.Length; i += 4)
        {
            buffer[i] = 0xFF; buffer[i + 1] = 0xFF; buffer[i + 2] = 0xFF; buffer[i + 3] = 0xFF;
        }

        // Paint dark modules.
        for (var y = 0; y < size; y++)
        {
            for (var x = 0; x < size; x++)
            {
                if (matrix.modules[y * size + x] != '1') continue;
                for (var dy = 0; dy < module; dy++)
                {
                    var row = (y * module + dy) * stride;
                    for (var dx = 0; dx < module; dx++)
                    {
                        var idx = row + (x * module + dx) * 4;
                        buffer[idx] = 0; buffer[idx + 1] = 0; buffer[idx + 2] = 0; buffer[idx + 3] = 0xFF;
                    }
                }
            }
        }

        bmp.WritePixels(new Int32Rect(0, 0, pixelSize, pixelSize), buffer, stride, 0);
        bmp.Freeze();
        _image.Source = bmp;
    }
}
