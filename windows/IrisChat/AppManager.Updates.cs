using System;
using System.Diagnostics;
using System.Windows;
using System.Windows.Threading;
using IrisChat.Bindings;

namespace IrisChat;

// The Rust core runs the update and words every step; this shell keeps the
// two preferences, schedules automatic checks, and starts the installer
// when the core says the update is ready.
public sealed partial class AppManager
{
    private static readonly TimeSpan DesktopUpdatePollInterval = LoadDesktopUpdatePollInterval();

    private readonly FfiDesktopUpdater? _updater;
    private DesktopUpdateView _updateView;
    private DispatcherTimer? _updatePollTimer;
    private bool _startupUpdateCheckDone;
    private bool _updateHandoffStarted;
    private string? _lastUpdateDiagnostic;
    private bool _autoCheckUpdates = UpdateService.LoadAutoCheckUpdates();
    private bool _autoInstallUpdates = UpdateService.LoadAutoInstallUpdates();

    /// What the Updates page shows: status, detail, progress and the button.
    public DesktopUpdateView UpdateView
    {
        get => _updateView;
        private set
        {
            _updateView = value;
            Notify(nameof(UpdateView));
        }
    }

    public bool AutoCheckUpdates
    {
        get => _autoCheckUpdates;
        set
        {
            if (!SetField(ref _autoCheckUpdates, value)) return;
            UpdateService.SaveAutoCheckUpdates(value);
            if (value)
            {
                StartDesktopUpdateChecks();
            }
            else
            {
                StopDesktopUpdateChecks();
            }
        }
    }

    public bool AutoInstallUpdates
    {
        get => _autoInstallUpdates;
        set
        {
            if (!SetField(ref _autoInstallUpdates, value)) return;
            UpdateService.SaveAutoInstallUpdates(value);
            if (value && UpdateView.step == DesktopUpdateStep.Available)
            {
                _updater?.Install();
            }
        }
    }

    public void StartDesktopUpdateChecks()
    {
        if (!AutoCheckUpdates)
        {
            StopDesktopUpdateChecks();
            return;
        }
        if (!_startupUpdateCheckDone)
        {
            _startupUpdateCheckDone = true;
            CheckForUpdates(manual: false);
        }
        if (_updatePollTimer is not null) return;

        _updatePollTimer = new DispatcherTimer { Interval = DesktopUpdatePollInterval };
        _updatePollTimer.Tick += (_, _) =>
        {
            if (!AutoCheckUpdates)
            {
                StopDesktopUpdateChecks();
                return;
            }
            CheckForUpdates(manual: false);
        };
        _updatePollTimer.Start();
    }

    private void StopDesktopUpdateChecks()
    {
        _updatePollTimer?.Stop();
        _updatePollTimer = null;
    }

    public void CheckForUpdates(bool manual = true)
    {
        try
        {
            _updater?.Check(manual, AutoInstallUpdates);
        }
        catch (Exception error)
        {
            LogFfiFailure("desktop_update.check", error);
        }
    }

    /// The Updates page button: Install, or Retry the step that failed.
    public void PerformUpdateAction()
    {
        try
        {
            _updater?.PerformAction();
        }
        catch (Exception error)
        {
            LogFfiFailure("desktop_update.action", error);
        }
    }

    /// Show a view from the core. Theme tests also call this to preview steps.
    public void ApplyUpdateView(DesktopUpdateView view)
    {
        UpdateView = view;
        if (view.step == DesktopUpdateStep.Failed
            && !string.IsNullOrWhiteSpace(view.diagnostic)
            && view.diagnostic != _lastUpdateDiagnostic)
        {
            _lastUpdateDiagnostic = view.diagnostic;
            AppendClientDebugLog("updates.failed", view.diagnostic);
        }
        if (view.quitToFinish && !_updateHandoffStarted)
        {
            _updateHandoffStarted = true;
            StartInstallerAndQuit(view.installerPath);
        }
    }

    // The installer replaces files this process holds open, so the app
    // closes once the installer has started. If it cannot start, the core
    // shows why and offers Retry.
    private void StartInstallerAndQuit(string? installerPath)
    {
        if (UpdateService.SkipOpen) return;
        try
        {
            if (string.IsNullOrWhiteSpace(installerPath))
            {
                throw new InvalidOperationException("The installer was not found.");
            }
            _ = Process.Start(new ProcessStartInfo(installerPath) { UseShellExecute = true });
        }
        catch (Exception error)
        {
            _updateHandoffStarted = false;
            _updater?.InstallerFailed(error.Message);
            return;
        }
        Application.Current.Shutdown();
    }

    private FfiDesktopUpdater? CreateUpdaterSafely()
    {
        try
        {
            return new FfiDesktopUpdater(new UpdateObserver(this), _dataDir, null);
        }
        catch (Exception error)
        {
            LogFfiFailure("desktop_update.new", error);
            return null;
        }
    }

    private DesktopUpdateView SafeUpdateView()
    {
        try
        {
            return _updater?.View() ?? Native.DesktopUpdateView(
                new DesktopUpdateSnapshot(new DesktopUpdateStage.Idle(), null, null));
        }
        catch (Exception error)
        {
            LogFfiFailure("desktop_update.view", error);
            return new DesktopUpdateView(DesktopUpdateStep.Idle, "", "", null, null, false, null,
                false, false, false, null, null, null);
        }
    }

    private static TimeSpan LoadDesktopUpdatePollInterval()
    {
        var raw = Environment.GetEnvironmentVariable("IRIS_UPDATE_POLL_SECONDS");
        return double.TryParse(raw, out var seconds) && seconds > 0
            ? TimeSpan.FromSeconds(seconds)
            : TimeSpan.FromHours(6);
    }

    private sealed class UpdateObserver : DesktopUpdateObserver
    {
        private readonly AppManager _owner;
        public UpdateObserver(AppManager owner) { _owner = owner; }

        // Called on the updater's thread; the dispatcher keeps the order.
        public void DesktopUpdateChanged(DesktopUpdateView view)
        {
            _owner._ui.BeginInvoke(new Action(() => _owner.ApplyUpdateView(view)));
        }
    }
}
