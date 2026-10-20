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
        string OnAccent,
        string Danger,
        string DangerSubtle,
        string ControlHover,
        string ControlPressed,
        string AccentText);

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
        "#FFFFFFFF",
        "#FFB4233D",
        "#14B4233D",
        "#0F000000",
        "#1F000000",
        "#FF702ACE");

    private static readonly PlatformPalette DarkPalette = new(
        "#FF000000",
        "#FF161616",
        "#FF262626",
        "#1FFFFFFF",
        "#F50A0A0A",
        "#FF702ACE",
        "#FF3A3A3A",
        "#FF702ACE",
        "#FFDB8216",
        "#FFFFFFFF",
        "#FFD1D5DB",
        "#FFFFFFFF",
        "#FFFF7485",
        "#33D63950",
        "#14FFFFFF",
        "#24FFFFFF",
        "#FFB47AF0");

    protected override void OnStartup(StartupEventArgs e)
    {
        // Register before singleton arbitration: a COM-launched secondary must
        // stay alive long enough to receive and forward the actual toast target.
        SystemDesktopNotificationPoster.Register(payload => Dispatcher.BeginInvoke(new Action(() =>
        {
            if (_singleInstance is null)
            {
                SingleInstanceService.ForwardToPrimary(new[] { DesktopNotificationNavigation.LaunchArgument, payload });
                Shutdown();
            }
            else
            {
                Manager.OpenNotification(payload);
                ShowMainWindow();
            }
        })));
        var toastActivated = SystemDesktopNotificationPoster.WasToastActivated;
        _singleInstance = SingleInstanceService.ClaimOrSignal(e.Args, signalExisting: !toastActivated);
        if (_singleInstance is null)
        {
            if (toastActivated) { ShutdownMode = ShutdownMode.OnExplicitShutdown; return; }
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

        var testRunId = Environment.GetEnvironmentVariable("IRIS_UI_TEST_RUN_ID");
        var requestedTestDataDir = Environment.GetEnvironmentVariable("IRIS_UI_TEST_DATA_DIR");
        var usesIsolatedTestData = !string.IsNullOrWhiteSpace(testRunId)
            && !string.IsNullOrWhiteSpace(requestedTestDataDir);
        var dataDir = usesIsolatedTestData
            ? Path.GetFullPath(requestedTestDataDir!)
            : Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "iris-chat"
            );
        Directory.CreateDirectory(dataDir);

        Manager = new AppManager(
            dataDir,
            testSecretPath: usesIsolatedTestData
                ? Path.Combine(dataDir, "account-secret.json")
                : null
        );
        ApplyPlatformPalette();
        SystemEvents.UserPreferenceChanged += OnUserPreferenceChanged;
        var startMinimized = IsBackgroundLaunch(e.Args);
        var window = new MainWindow(dataDir, startMinimized);
        _window = window;
        MainWindow = window;
        _singleInstance.Start(args => Dispatcher.Invoke(() => HandleLaunchArgs(args)));
        window.Show();
        HandleLaunchArgs(e.Args);
    }

    protected override void OnExit(ExitEventArgs e)
    {
        SystemEvents.UserPreferenceChanged -= OnUserPreferenceChanged;
        _singleInstance?.Dispose();
        Manager?.Shutdown();
        base.OnExit(e);
    }

    public static AppManager CurrentManager =>
        ((App)Current).Manager;

    private void HandleLaunchArgs(IEnumerable<string> args)
    {
        var values = args.ToArray();
        var index = Array.IndexOf(values, DesktopNotificationNavigation.LaunchArgument);
        if (index >= 0 && index + 1 < values.Length) Manager.OpenNotification(values[index + 1]);
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

    private void OnUserPreferenceChanged(object sender, UserPreferenceChangedEventArgs e)
    {
        if (!Dispatcher.HasShutdownStarted)
            Dispatcher.BeginInvoke(new Action(ApplyPlatformPalette));
    }

    private static void ApplyPlatformPalette() => ApplyPalette(UsesWindowsLightAppTheme());

    private static void ApplyPalette(bool light)
    {
        var palette = light ? LightPalette : DarkPalette;
        SetColor("Background", palette.Background);
        SetColor("Panel", palette.Panel);
        SetColor("PanelAlt", palette.PanelAlt);
        SetColor("Border", palette.Border);
        SetColor("Toolbar", palette.Toolbar);
        SetColor("BubbleMine", palette.BubbleMine);
        SetColor("BubbleTheirs", palette.BubbleTheirs);
        SetColor("Accent", palette.Accent);
        SetColor("AccentAlt", palette.AccentAlt);
        SetColor("TextPrimary", palette.TextPrimary);
        SetColor("TextMuted", palette.TextMuted);
        SetColor("OnAccent", palette.OnAccent);
        SetColor("Danger", palette.Danger);
        SetColor("DangerSubtle", palette.DangerSubtle);
        SetColor("ControlHover", palette.ControlHover);
        SetColor("ControlPressed", palette.ControlPressed);
        SetColor("AccentText", palette.AccentText);
        ((ThemeColors)Current.Resources["ThemeColors"]).Refresh();
    }

    private static bool UsesWindowsLightAppTheme()
    {
        const string personalize = @"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
        var value = Registry.GetValue(personalize, "AppsUseLightTheme", null);
        return value is not int intValue || intValue != 0;
    }

    private static void SetColor(string key, string color)
    {
        ((ThemeColors)Current.Resources["ThemeColors"])[key] = (Color)ColorConverter.ConvertFromString(color)!;
    }
}
