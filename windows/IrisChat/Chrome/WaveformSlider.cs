using System;
using System.Collections.Generic;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace IrisChat.Chrome;

// Retain Slider's range automation and keyboard semantics while drawing the
// decoded waveform instead of the platform's ordinary progress track.
public sealed class WaveformSlider : Slider
{
    private IReadOnlyList<float> _peaks = Array.Empty<float>();
    public IReadOnlyList<float> Peaks
    {
        get => _peaks;
        set { _peaks = value; InvalidateVisual(); }
    }

    public WaveformSlider()
    {
        Height = 32;
        Foreground = Brushes.White;
        Template = new ControlTemplate(typeof(Slider));
        ValueChanged += (_, _) => InvalidateVisual();
        IsEnabledChanged += (_, _) => InvalidateVisual();
    }

    protected override void OnRender(DrawingContext dc)
    {
        base.OnRender(dc);
        dc.DrawRectangle(Brushes.Transparent, null, new Rect(0, 0, ActualWidth, ActualHeight));
        var width = Math.Max(1, ActualWidth - 8);
        var step = width / AudioWaveform.BarCount;
        var fraction = Maximum > Minimum ? (Value - Minimum) / (Maximum - Minimum) : 0;
        for (var i = 0; i < AudioWaveform.BarCount; i++)
        {
            var height = 3 + (i < _peaks.Count ? _peaks[i] : 0) * 21;
            dc.PushOpacity((i + 0.5) / AudioWaveform.BarCount <= fraction ? 1 : 0.35);
            dc.DrawRoundedRectangle(Foreground, null, new Rect(4 + i * step, (ActualHeight - height) / 2, Math.Max(1, step - 1.5), height), 1, 1);
            dc.Pop();
        }
        if (IsEnabled)
            dc.DrawEllipse(Foreground, null, new Point(4 + fraction * width, ActualHeight / 2), 3, 3);
    }

    private void SeekAt(Point point) => Value = Minimum + Math.Clamp((point.X - 4) / Math.Max(1, ActualWidth - 8), 0, 1) * (Maximum - Minimum);

    protected override void OnMouseLeftButtonDown(MouseButtonEventArgs e)
    {
        Focus(); CaptureMouse(); SeekAt(e.GetPosition(this)); e.Handled = true;
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        if (IsMouseCaptured && e.LeftButton == MouseButtonState.Pressed) { SeekAt(e.GetPosition(this)); e.Handled = true; }
        else base.OnMouseMove(e);
    }

    protected override void OnMouseLeftButtonUp(MouseButtonEventArgs e)
    {
        if (IsMouseCaptured) { SeekAt(e.GetPosition(this)); ReleaseMouseCapture(); e.Handled = true; }
        else base.OnMouseLeftButtonUp(e);
    }

    protected override void OnKeyDown(KeyEventArgs e)
    {
        var step = Keyboard.Modifiers.HasFlag(ModifierKeys.Shift) ? 5 : 1;
        var next = e.Key switch
        {
            Key.Left or Key.Down => Value - step, Key.Right or Key.Up => Value + step,
            Key.Home => Minimum, Key.End => Maximum, _ => double.NaN,
        };
        if (double.IsNaN(next)) { base.OnKeyDown(e); return; }
        Value = Math.Clamp(next, Minimum, Maximum); e.Handled = true;
    }
}
