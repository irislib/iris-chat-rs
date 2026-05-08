using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Windows;
using System.Windows.Media;
using Microsoft.Win32;

namespace IrisChat;

public partial class App : Application
{
    private SingleInstanceService? _singleInstance;
    private MainWindow? _window;
    public AppManager Manager { get; private set; } = null!;

    private readonly record struct PlatformPalette(
        string Background,
        string Panel,
        string PanelAlt,
        string Border,
        string Toolbar,
        string BubbleMine,
        string BubbleTheirs,
        string Accent,
        string AccentAlt,
        string TextPrimary,
        string TextMuted,
        string OnAccent);

    private static readonly PlatformPalette LightPalette = new(
        "#FFFFFFFF",
        "#FFF7F9FA",
        "#FFE1E8ED",
        "#14000000",
        "#F5F7F9FA",
        "#FF702ACE",
        "#FFF7F9FA",
        "#FF702ACE",
        "#FFDB8216",
        "#FF0F1419",
        "#FF536471",
        "#FFFFFFFF");

    private static readonly PlatformPalette DarkPalette = new(
        "#FF101010",
        "#FF242424",
        "#FF343434",
        "#1FFFFFFF",
        "#F5181818",
        "#FF702ACE",
        "#FF3A3A3A",
        "#FF702ACE",
        "#FFDB8216",
        "#FFFFFFFF",
        "#FFD1D5DB",
        "#FFFFFFFF");

    protected override void OnStartup(StartupEventArgs e)
    {
        _singleInstance = SingleInstanceService.ClaimOrSignal(e.Args);
        if (_singleInstance is null)
        {
            Shutdown();
            return;
        }

        base.OnStartup(e);

        // Make the bundled native DLL discoverable to P/Invoke regardless of
        // whether the app was launched from publish/, bin/, or via dotnet run.
        var exeDir = AppContext.BaseDirectory;
        Environment.SetEnvironmentVariable(
            "PATH",
            $"{exeDir};{Environment.GetEnvironmentVariable("PATH")}"
        );

        var dataDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "iris-chat"
        );
        Directory.CreateDirectory(dataDir);

        Manager = new AppManager(dataDir);
        ApplyPlatformPalette();
        var startMinimized = IsBackgroundLaunch(e.Args);
        var window = new MainWindow(startMinimized);
        _window = window;
        MainWindow = window;
        _singleInstance.Start(args => Dispatcher.Invoke(() => HandleLaunchArgs(args)));
        window.Show();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        _singleInstance?.Dispose();
        Manager?.Shutdown();
        base.OnExit(e);
    }

    public static AppManager CurrentManager =>
        ((App)Current).Manager;

    private void HandleLaunchArgs(IEnumerable<string> args)
    {
        if (!IsBackgroundLaunch(args))
        {
            ShowMainWindow();
        }
    }

    private void ShowMainWindow()
    {
        if (_window is null)
        {
            return;
        }
        if (!_window.IsVisible)
        {
            _window.Show();
        }
        if (_window.WindowState == WindowState.Minimized)
        {
            _window.WindowState = WindowState.Normal;
        }
        _window.Activate();
    }

    private static bool IsBackgroundLaunch(IEnumerable<string> args) =>
        args.Any(arg =>
            string.Equals(arg, PlatformStartupAtLogin.BackgroundLaunchArgument, StringComparison.OrdinalIgnoreCase));

    private static void ApplyPlatformPalette()
    {
        var palette = UsesWindowsLightAppTheme() ? LightPalette : DarkPalette;
        SetBrush("Background", palette.Background);
        SetBrush("Panel", palette.Panel);
        SetBrush("PanelAlt", palette.PanelAlt);
        SetBrush("Border", palette.Border);
        SetBrush("Toolbar", palette.Toolbar);
        SetBrush("BubbleMine", palette.BubbleMine);
        SetBrush("BubbleTheirs", palette.BubbleTheirs);
        SetBrush("Accent", palette.Accent);
        SetBrush("AccentAlt", palette.AccentAlt);
        SetBrush("TextPrimary", palette.TextPrimary);
        SetBrush("TextMuted", palette.TextMuted);
        SetBrush("OnAccent", palette.OnAccent);
    }

    private static bool UsesWindowsLightAppTheme()
    {
        const string personalize = @"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
        var value = Registry.GetValue(personalize, "AppsUseLightTheme", null);
        return value is int intValue && intValue != 0;
    }

    private static void SetBrush(string key, string color)
    {
        var colorValue = (Color)ColorConverter.ConvertFromString(color)!;
        if (Current.Resources[key] is SolidColorBrush brush && !brush.IsFrozen)
        {
            brush.Color = colorValue;
            return;
        }
        Current.Resources[key] = new SolidColorBrush(colorValue);
    }
}
