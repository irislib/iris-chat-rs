using System;
using System.IO;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Views;

// The Rust core runs the update and words each step (core/src/desktop_update).
// These checks show that the Updates page renders every view it sends.
internal static class UpdateChecks
{
    private const string Tag = "v2026.11.17";

    internal static void Run(Window window, AppManager manager, string output)
    {
        Environment.SetEnvironmentVariable("IRIS_UPDATE_SKIP_OPEN", "1");
        var settings = new SettingsView();
        Program.Show(window, settings);
        var updates = Program.Descendants<ListBoxItem>(settings).Single(item => (item.Tag as string) == "Updates");
        updates.IsSelected = true; Program.Pump();
        var status = (TextBlock)settings.FindName("UpdateStatusText");
        var detail = (TextBlock)settings.FindName("UpdateDetailText");
        var progress = (ProgressBar)settings.FindName("UpdateProgress");
        var action = (Button)settings.FindName("InstallUpdateButton");
        var actionText = (TextBlock)settings.FindName("InstallUpdateButtonText");
        var check = (Button)settings.FindName("CheckUpdatesButton");

        RealFailureShowsRetry(manager, status, detail, action, actionText);

        var steps = new (string Name, DesktopUpdateStage Stage)[]
        {
            ("available", new DesktopUpdateStage.Available(Tag)),
            ("fetching", new DesktopUpdateStage.Fetching(Tag)),
            ("downloading", new DesktopUpdateStage.Downloading(Tag, 12_400_000UL, 36_200_000UL)),
            ("verifying", new DesktopUpdateStage.Verifying(Tag)),
            ("opening-installer", new DesktopUpdateStage.Restarting(Tag, @"C:\Temp\iris-chat-setup.exe")),
            ("failed-download-stalled", Failed(DesktopUpdateFailure.DownloadStalled, null)),
            ("failed-installer", Failed(DesktopUpdateFailure.InstallerDidNotStart, "The operation was canceled by the user.")),
            ("failed-did-not-finish", Failed(DesktopUpdateFailure.DidNotFinish, "v2026.11.13.6")),
        };
        foreach (var light in new[] { true, false })
        {
            Program.SetPalette(light); Program.Pump();
            var theme = light ? "light" : "dark";
            foreach (var (name, stage) in steps)
            {
                var view = Native.DesktopUpdateView(new DesktopUpdateSnapshot(stage, 1_794_900_000UL, null));
                manager.ApplyUpdateView(view);
                Program.Pump();
                Program.Check(status.Text == view.status && !string.IsNullOrEmpty(status.Text), $"{name}: status shown");
                Program.Check(detail.Text == (view.detail ?? ""), $"{name}: detail shown");
                if (view.busy)
                {
                    Program.Check(progress.IsVisible, $"{name}: progress shown while busy");
                    Program.Check(progress.IsIndeterminate == (view.progress is null), $"{name}: bar matches known size");
                    Program.Check(!action.IsVisible && !check.IsEnabled, $"{name}: buttons wait while busy");
                }
                else
                {
                    Program.Check(!progress.IsVisible, $"{name}: no progress when idle");
                    Program.Check(action.IsVisible && actionText.Text == view.actionLabel, $"{name}: {view.actionLabel} offered");
                }
                if (name.StartsWith("failed", StringComparison.Ordinal))
                {
                    Program.Check(actionText.Text == "Retry", $"{name}: Retry offered");
                }
                Contrast.CheckTree(window, $"{theme} update {name}");
                Program.Save(window, Path.Combine(output, $"{theme}-update-{name}.png"));
            }
        }
        Program.Check(Application.Current.Windows.Cast<Window>().Contains(window),
            "Opening the installer is skipped in tests and leaves the app open");
        Console.WriteLine($"PASS: update page shows {steps.Length} steps and failures with progress and Retry");
    }

    // A missing release file makes the real core fail; the page says so and
    // offers Retry instead of re-enabling a silent button.
    private static void RealFailureShowsRetry(AppManager manager, TextBlock status, TextBlock detail,
        Button action, TextBlock actionText)
    {
        var missing = new Uri(Path.Combine(Path.GetTempPath(), "iris-missing-" + Guid.NewGuid().ToString("N"), "release.json"));
        Environment.SetEnvironmentVariable("IRIS_UPDATE_MANIFEST_URL", missing.AbsoluteUri);
        try
        {
            manager.CheckForUpdates();
            Program.PumpUntil(() => manager.UpdateView.step == DesktopUpdateStep.Failed);
            Program.Pump();
            Program.Check(status.Text == "Couldn’t check for updates", "check failure is named");
            Program.Check(detail.Text == "Check your connection and try again.", "check failure says what to do");
            Program.Check(action.IsVisible && actionText.Text == "Retry", "check failure offers Retry");
            Console.WriteLine("PASS: a failed update check from the core is shown with Retry");
        }
        finally
        {
            Environment.SetEnvironmentVariable("IRIS_UPDATE_MANIFEST_URL", null);
        }
    }

    private static DesktopUpdateStage Failed(DesktopUpdateFailure failure, string? context) =>
        new DesktopUpdateStage.Failed(Tag, failure, context);
}
