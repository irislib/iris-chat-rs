using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Windows;
using Microsoft.Win32;

namespace IrisChat;

public static class PlatformClipboard
{
    public static string? GetString()
    {
        try { return Clipboard.ContainsText() ? Clipboard.GetText() : null; }
        catch { return null; }
    }

    public static void SetString(string value)
    {
        try { Clipboard.SetText(value ?? string.Empty); } catch { }
    }
}

public static class PlatformDocumentOpener
{
    public static bool Open(string path)
    {
        try
        {
            Process.Start(new ProcessStartInfo
            {
                FileName = path,
                UseShellExecute = true,
            });
            return true;
        }
        catch
        {
            return false;
        }
    }

    public static bool OpenUrl(string url)
    {
        try
        {
            Process.Start(new ProcessStartInfo { FileName = url, UseShellExecute = true });
            return true;
        }
        catch
        {
            return false;
        }
    }
}

public static class PlatformFilePicker
{
    public static string[]? PickFiles(string title, bool multiselect = false, string? filter = null)
    {
        var dialog = new OpenFileDialog
        {
            Title = title,
            Multiselect = multiselect,
            Filter = filter ?? "All files (*.*)|*.*",
        };
        return dialog.ShowDialog() == true ? dialog.FileNames : null;
    }

    public static string? PickImage(string title)
    {
        var files = PickFiles(
            title,
            multiselect: false,
            filter: "Images (*.png;*.jpg;*.jpeg;*.gif;*.webp;*.bmp)|*.png;*.jpg;*.jpeg;*.gif;*.webp;*.bmp|All files (*.*)|*.*"
        );
        return files?.FirstOrDefault();
    }

    public static string? SaveFile(string title, string suggestedName, string filter)
    {
        var dialog = new SaveFileDialog
        {
            Title = title,
            FileName = suggestedName,
            Filter = filter,
        };
        return dialog.ShowDialog() == true ? dialog.FileName : null;
    }
}

public static class PlatformDeviceLabels
{
    public static string CurrentDeviceLabel
    {
        get
        {
            var name = Environment.MachineName?.Trim();
            return string.IsNullOrEmpty(name) ? "Windows PC" : name!;
        }
    }

    public static string CurrentClientLabel => "Iris Chat Desktop";
}

public static class PlatformStartupAtLogin
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "IrisChat";
    public const string BackgroundLaunchArgument = "--background";

    public static bool IsSupported => true;

    public static bool IsEnabled
    {
        get
        {
            try
            {
                using var key = Registry.CurrentUser.OpenSubKey(RunKey, writable: false);
                return key?.GetValue(ValueName) is string s && s.Length > 0;
            }
            catch { return false; }
        }
    }

    public static void SetEnabled(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(RunKey, writable: true)
            ?? throw new InvalidOperationException("Could not open Run key");
        if (enabled)
        {
            var exePath = Process.GetCurrentProcess().MainModule?.FileName;
            if (string.IsNullOrEmpty(exePath))
            {
                throw new InvalidOperationException("Cannot resolve current executable path");
            }
            key.SetValue(ValueName, $"\"{exePath}\" {BackgroundLaunchArgument}", RegistryValueKind.String);
        }
        else
        {
            key.DeleteValue(ValueName, throwOnMissingValue: false);
        }
    }
}

public interface IDesktopNotificationPoster
{
    void Post(string title, string body);
}

/// Best-effort Windows notification poster. Uses WPF + balloon-via-shell as a
/// simple fallback. Production-grade toasts would use Windows.UI.Notifications,
/// which requires Windows.winmd interop — out of scope for the first version.
public sealed class SystemDesktopNotificationPoster : IDesktopNotificationPoster
{
    public void Post(string title, string body)
    {
        try
        {
            var notify = new System.Windows.Forms.NotifyIcon
            {
                Icon = System.Drawing.SystemIcons.Information,
                Visible = true,
                BalloonTipTitle = title,
                BalloonTipText = body,
            };
            notify.ShowBalloonTip(4000);
            // Dispose after the balloon has had a chance to show.
            var timer = new System.Windows.Threading.DispatcherTimer
            {
                Interval = TimeSpan.FromSeconds(6),
            };
            timer.Tick += (_, _) => { timer.Stop(); notify.Dispose(); };
            timer.Start();
        }
        catch
        {
            // Notifications are best-effort.
        }
    }
}
