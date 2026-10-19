using System;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Chrome;

/// Reserve two lines using the current font metrics, including empty previews.
public sealed class TwoLinePreview : TextBlock
{
    public TwoLinePreview()
    {
        TextWrapping = TextWrapping.Wrap;
        TextTrimming = TextTrimming.CharacterEllipsis;
        LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        UpdateLineSpace();
    }
    protected override void OnPropertyChanged(DependencyPropertyChangedEventArgs e)
    {
        base.OnPropertyChanged(e);
        if (e.Property == FontSizeProperty || e.Property == FontFamilyProperty) UpdateLineSpace();
    }
    private void UpdateLineSpace()
    {
        LineHeight = Math.Max(1, Math.Ceiling(FontSize * FontFamily.LineSpacing));
        Height = 2 * LineHeight;
    }
}
