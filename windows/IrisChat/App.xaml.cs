using System;
using System.IO;
using System.Linq;
using System.Windows;

namespace IrisChat;

public partial class App : Application
{
    public AppManager Manager { get; private set; } = null!;

    protected override void OnStartup(StartupEventArgs e)
    {
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
        var startMinimized = e.Args.Any(arg =>
            string.Equals(arg, PlatformStartupAtLogin.BackgroundLaunchArgument, StringComparison.OrdinalIgnoreCase));
        var window = new MainWindow(startMinimized);
        MainWindow = window;
        window.Show();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        Manager?.Shutdown();
        base.OnExit(e);
    }

    public static AppManager CurrentManager =>
        ((App)Current).Manager;
}
