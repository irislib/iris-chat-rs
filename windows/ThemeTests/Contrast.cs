using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using IrisChat.Chrome;

internal static class Contrast
{
    internal static int Count;
    internal static void CheckTree(Window window, string context)
    {
        var checkedHere = 0;
        foreach (var element in Program.Descendants<FrameworkElement>(window))
        {
            if (!element.IsVisible || !element.IsEnabled) continue;
            Brush? foreground = null;
            string text = "";
            var minimum = 4.5;
            switch (element)
            {
                case TextBlock label when !string.IsNullOrWhiteSpace(label.Text):
                    foreground = label.Foreground; text = label.Text;
                    if (label.FontFamily.Source.Contains("MDL2")) minimum = 3;
                    break;
                case TextBox input:
                    foreground = input.Foreground; text = "input " + input.Name;
                    break;
                case PasswordBox password:
                    foreground = password.Foreground; text = "password input";
                    break;
                case RichTextBox rich when rich.Visibility == Visibility.Visible:
                    foreground = rich.Foreground; text = "message text";
                    break;
                case WaveformSlider waveform:
                    foreground = waveform.Foreground; text = "audio waveform"; minimum = 3;
                    break;
            }
            if (foreground is not SolidColorBrush brush) continue;
            var background = Background(element);
            var opacity = Ancestors(element).OfType<UIElement>().Aggregate(element.Opacity, (value, parent) => value * parent.Opacity);
            var ink = Over(brush.Color, background, opacity * brush.Opacity);
            var ratio = Ratio(ink, background);
            Program.Check(ratio >= minimum, $"{context}: '{text[..Math.Min(60, text.Length)]}' has {ratio:F2}:1 contrast ({ink} on {background}), expected {minimum}:1");
            checkedHere++; Count++;
        }
        Program.Check(checkedHere > 0, context + " contains rendered text");
    }

    private static IEnumerable<DependencyObject> Ancestors(DependencyObject child)
    {
        for (var parent = VisualTreeHelper.GetParent(child); parent != null; parent = VisualTreeHelper.GetParent(parent)) yield return parent;
    }
    private static Color Background(FrameworkElement element)
    {
        // Avatar initials overlay a sibling background rather than an ancestor.
        if (element.Name == "Initials" && Ancestors(element).OfType<Avatar>().FirstOrDefault() is {} avatar)
            return ((SolidColorBrush)((Border)avatar.FindName("BackgroundBorder")).Background).Color;
        var result = Colors.White;
        foreach (var node in Ancestors(element).Reverse().Append(element))
        {
            var brush = node switch {
                Border border => border.Background,
                Panel panel => panel.Background,
                // Controls paint through their visual templates. For example,
                // CheckBox.Background is white by default but our switch does
                // not draw it; its track is a separate Border.
                Window window => window.Background,
                TextBlock label => label.Background,
                _ => null,
            };
            if (brush is SolidColorBrush solid)
                result = Over(solid.Color, result, solid.Opacity);
        }
        return result;
    }
    private static Color Over(Color ink, Color paper, double opacity)
    {
        var alpha = ink.A / 255.0 * opacity;
        byte Mix(byte a, byte b) => (byte)Math.Round(a * alpha + b * (1 - alpha));
        return Color.FromRgb(Mix(ink.R, paper.R), Mix(ink.G, paper.G), Mix(ink.B, paper.B));
    }
    private static double Ratio(Color first, Color second)
    {
        static double Luminance(Color color)
        {
            static double Linear(byte channel) { var value = channel / 255.0; return value <= 0.04045 ? value / 12.92 : Math.Pow((value + 0.055) / 1.055, 2.4); }
            return 0.2126 * Linear(color.R) + 0.7152 * Linear(color.G) + 0.0722 * Linear(color.B);
        }
        var a = Luminance(first); var b = Luminance(second);
        return (Math.Max(a, b) + 0.05) / (Math.Min(a, b) + 0.05);
    }
}
