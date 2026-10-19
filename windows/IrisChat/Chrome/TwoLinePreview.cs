using System;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Chrome;

/// Reserve two lines using the current font metrics, including empty previews.
public sealed class TwoLinePreview : TextBlock
{
    static TwoLinePreview()
    {
        FontSizeProperty.OverrideMetadata(typeof(TwoLinePreview), new FrameworkPropertyMetadata(OnFontMetricsChanged));
        FontFamilyProperty.OverrideMetadata(typeof(TwoLinePreview), new FrameworkPropertyMetadata(OnFontMetricsChanged));
    }

    public TwoLinePreview()
    {
        TextWrapping = TextWrapping.Wrap;
        TextTrimming = TextTrimming.CharacterEllipsis;
        LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        UpdateLineSpace();
    }
    private static void OnFontMetricsChanged(DependencyObject target, DependencyPropertyChangedEventArgs e) =>
        ((TwoLinePreview)target).UpdateLineSpace();
    private void UpdateLineSpace()
    {
        LineHeight = Math.Max(1, Math.Ceiling(FontSize * FontFamily.LineSpacing));
        Height = 2 * LineHeight;
    }
}
