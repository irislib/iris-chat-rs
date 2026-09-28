using System;
using System.Globalization;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Media;

namespace IrisChat.Chrome;

public sealed class AudioMessageControl : UserControl
{
    private readonly Button _play = new() { Width = 44, Height = 44, FontSize = 20, Content = "▶", Padding = new Thickness(0), Foreground = Brushes.White };
    private readonly WaveformSlider _progress = new() { Minimum = 0, Maximum = 1, IsEnabled = false, IsMoveToPointEnabled = true };
    private readonly TextBlock _time = new() { Text = "Audio", FontSize = 11, Opacity = 0.7, Foreground = Brushes.White, TextWrapping = TextWrapping.Wrap };
    private readonly Button _speed = new() { Content = "1×", FontSize = 11, Foreground = Brushes.White, Padding = new Thickness(6, 2, 6, 2) };
    private AudioMessagePlayback? _playback;
    private bool _updating;

    public AudioMessageControl(string key, string filename, Func<Task<byte[]?>> download)
    {
        Width = 244;
        Foreground = Brushes.White;
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(54) });
        grid.ColumnDefinitions.Add(new ColumnDefinition());
        _play.SetResourceReference(StyleProperty, "GhostButton");
        _speed.SetResourceReference(StyleProperty, "GhostButton");
        grid.Children.Add(_play);
        var right = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        Grid.SetColumn(right, 1);
        right.Children.Add(_progress);
        var footer = new DockPanel();
        DockPanel.SetDock(_speed, Dock.Right);
        footer.Children.Add(_speed);
        footer.Children.Add(_time);
        right.Children.Add(footer);
        grid.Children.Add(right);
        Content = grid;
        AutomationProperties.SetName(_progress, "Audio position");
        AutomationProperties.SetName(_speed, "Playback speed");
        AutomationProperties.SetAutomationId(_play, "chatAudioPlayButton");
        AutomationProperties.SetAutomationId(_progress, "chatAudioProgress");
        AutomationProperties.SetAutomationId(_speed, "chatAudioSpeedButton");
        _play.Click += async (_, _) => { if (_playback != null) await _playback.ToggleAsync(); };
        _speed.Click += (_, _) => _playback?.CycleRate();
        _progress.ValueChanged += (_, _) => { if (!_updating) _playback?.Seek(_progress.Value); };
        Loaded += (_, _) =>
        {
            if (_playback != null) return;
            _playback = AudioMessagePlayback.Attach(key, filename, download);
            _playback.Changed += Refresh;
            Refresh();
        };
        Unloaded += (_, _) =>
        {
            if (_playback == null) return;
            _playback.Changed -= Refresh;
            _playback.Detach();
            _playback = null;
        };
        AutomationProperties.SetName(_play, "Play audio");
    }

    private void Refresh()
    {
        if (_playback == null) return;
        _updating = true;
        _play.Content = _playback.IsLoading ? "…" : _playback.Error != null ? "↻" : _playback.IsPlaying ? "Ⅱ" : "▶";
        var action = _playback.IsLoading ? "Cancel loading" : _playback.Error != null ? "Retry audio" : _playback.IsPlaying ? "Pause audio" : "Play audio";
        AutomationProperties.SetName(_play, action);
        _play.ToolTip = action;
        _progress.Peaks = _playback.Peaks;
        _progress.Maximum = Math.Max(1, _playback.Duration);
        _progress.Value = Math.Clamp(_playback.Elapsed, 0, _progress.Maximum);
        _progress.IsEnabled = _playback.Duration > 0 && !_playback.IsLoading && _playback.Error == null;
        _time.Text = _playback.Error ?? (_playback.Duration > 0 ? $"{Time(_playback.Elapsed)} / {Time(_playback.Duration)}" : "Audio");
        _speed.Content = _playback.Rate.ToString("0.#", CultureInfo.InvariantCulture) + "×";
        _speed.ToolTip = $"Playback speed: {_speed.Content}";
        _updating = false;
    }

    private static string Time(double value) => $"{(int)value / 60}:{(int)value % 60:00}";
}
