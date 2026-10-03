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
    public const string MediaFilter = "Photos and videos|*.jpg;*.jpeg;*.png;*.gif;*.webp;*.bmp;*.heic;*.heif;*.avif;*.tif;*.tiff;*.mp4;*.m4v;*.mov;*.webm;*.mkv;*.avi;*.mpeg;*.mpg;*.3gp";

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
            var device = string.IsNullOrEmpty(name) ? "Windows PC" : name!;
            var os = RuntimeInformation.OSDescription.Trim();
            return string.IsNullOrEmpty(os) ? device : $"{device} - {os}";
        }
    }

    public static string CurrentClientLabel => "Iris Chat Windows";
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
    void Post(string title, string body, DesktopNotificationTarget target);
    void Clear();
}
