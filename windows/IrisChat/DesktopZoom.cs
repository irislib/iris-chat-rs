using System;
using System.Globalization;
using System.IO;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Input;
using System.Windows.Media;

namespace IrisChat;

internal sealed class DesktopZoom
{
    private readonly FrameworkElement _content;
    private readonly string _settingsPath;
    private int _level;
    private Task _save = Task.CompletedTask;

    internal DesktopZoom(Window window, FrameworkElement content, string dataDir)
    {
        _content = content;
        _settingsPath = Path.Combine(dataDir, "desktop-zoom.txt");
        try
        {
            if (int.TryParse(File.ReadAllText(_settingsPath), NumberStyles.Integer,
                CultureInfo.InvariantCulture, out var saved)) _level = Math.Clamp(saved, -3, 4);
        }
        catch (IOException) { }
        catch (UnauthorizedAccessException) { }
        Apply();
        window.PreviewKeyDown += OnKeyDown;
    }

    private void OnKeyDown(object sender, KeyEventArgs e)
    {
        var modifiers = Keyboard.Modifiers;
        if ((modifiers & ModifierKeys.Control) == 0 ||
            (modifiers & (ModifierKeys.Alt | ModifierKeys.Windows)) != 0) return;
        var next = e.Key switch
        {
            Key.OemPlus or Key.Add => Math.Min(4, _level + 1),
            Key.OemMinus or Key.Subtract => Math.Max(-3, _level - 1),
            Key.D0 or Key.NumPad0 => 0,
            _ => (int?)null,
        };
        if (next == null) return;
        e.Handled = true;
        _level = next.Value;
        Apply();
        var saved = _level.ToString(CultureInfo.InvariantCulture);
        _save = _save.ContinueWith(_ =>
        {
            try { File.WriteAllText(_settingsPath, saved); }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }, TaskScheduler.Default);
    }

    private void Apply()
    {
        var scale = Math.Pow(1.2, _level);
        _content.LayoutTransform = new ScaleTransform(scale, scale);
    }
}
