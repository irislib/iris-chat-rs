using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Threading;
using IrisChat.Bindings;

namespace IrisChat;

/// Windows shell-side AppManager. Mirrors the iOS/macOS AppManager.swift
/// contract: build the Rust core, restore secure credentials, render Rust
/// state, dispatch actions, persist secret side effects.
public sealed partial class AppManager : INotifyPropertyChanged
{
    private const string CoreRestartToast = "Iris needs restart. Copy support bundle in Settings.";
    private const string DispatchFailureToast = "Action failed. Copy support bundle in Settings.";
    private const int MaxClientDebugLogDetailChars = 1000;
    private const int MaxClientDebugLogEntries = 50;
    private const uint RouteChatSnapshotLimit = 80;
    private static readonly TimeSpan ActiveChatSeenIdleLimit = TimeSpan.FromMinutes(5);
    private static readonly TimeSpan NavigationOverrideTtl = TimeSpan.FromSeconds(10);
    private static readonly TimeSpan DesktopUpdatePollInterval = LoadDesktopUpdatePollInterval();

    private sealed record PendingNavigationOverride(Screen[] Stack, DateTimeOffset ExpiresAt);
    private sealed record ClientDebugLogEntry(long TimestampSecs, string Category, string Detail);

    public event PropertyChangedEventHandler? PropertyChanged;

    private readonly FfiApp _ffi;
    private readonly WindowsCredentialStore _secretStore;
    private readonly WindowsCredentialStore _pendingDeviceLinkSecretStore;
    private readonly IDesktopNotificationPoster _notifier;
    private readonly HashtreeAttachmentCache _cache;
    private readonly UpdateService _updateService = new();
    private readonly string _dataDir;
    private readonly FfiDesktopNearby? _nearby;
    private readonly Dispatcher _ui;
    private readonly object _toastLock = new();
    private readonly object _clientDebugLogLock = new();
    private readonly List<ClientDebugLogEntry> _clientDebugLog = new();
    private DispatcherTimer? _updatePollTimer;
    private string? _activeToast;
    private bool _updateHasAsset;
    private bool _startupUpdateCheckDone;
    private bool _updateChecking;
    private bool _updateInstalling;
    private bool _updateAvailable;
    private bool _autoCheckUpdates = UpdateService.LoadAutoCheckUpdates();
    private bool _autoInstallUpdates = UpdateService.LoadAutoInstallUpdates();
    private DateTimeOffset _lastUserActivityUtc = DateTimeOffset.UtcNow;
    private string _updateVersion = "";
    private string _updateStatus = "";

    private AppState _state;
    private DesktopNearbySnapshot _nearbySnapshot;
    private ulong _lastRevApplied;
    private bool _persistedRestoreInFlight;
    private bool _automaticRevocationLogoutInFlight;
    private PendingNavigationOverride? _pendingNavigationOverride;
    private string? _lastSyncedDeviceLabelsKey;

    public AppManager(string dataDir, IDesktopNotificationPoster? notifier = null, string? testSecretPath = null)
    {
        _dataDir = dataDir;
        _secretStore = new WindowsCredentialStore(filePath: testSecretPath);
        _pendingDeviceLinkSecretStore = new WindowsCredentialStore(
            targetName: "to.iris.chat/pending-device-link",
            filePath: testSecretPath is null ? null : testSecretPath + ".pending-link"
        );
        _notifier = notifier ?? new SystemDesktopNotificationPoster();
        _cache = new HashtreeAttachmentCache(dataDir);
        _ui = Application.Current.Dispatcher;

        var version = typeof(AppManager).Assembly.GetName().Version?.ToString(3) ?? "0.1.0";
        _ffi = new FfiApp(dataDir, "", version);
        _state = SafeState();
        _lastRevApplied = _state.rev;
        _nearby = CreateNearbySafely();
        _nearbySnapshot = SafeNearbySnapshot();

        ListenForUpdatesSafely();
        if (!LogoutIfCurrentDeviceRevoked(_state))
        {
            SyncCurrentDeviceLabels(_state);
            SyncStartupAtLoginPreference();
            TryRestorePersistedSession();
        }
        StartDesktopUpdateChecks();
    }

    // ────────────────────────────── projection ────────────────────────────────

    public AppState State => _state;
    public bool BootstrapInFlight { get; private set; } = true;
    public string? ToastMessage => _activeToast;

    public Screen ActiveScreen =>
        _state.router.screenStack.LastOrDefault() ?? _state.router.defaultScreen;

    public bool CanNavigateBack => (_state.router.screenStack?.Length ?? 0) > 0;

    public AccountSnapshot? Account => _state.account;
    public DeviceRosterSnapshot? DeviceRoster => _state.deviceRoster;
    public ChatThreadSnapshot[] ChatList => _state.chatList ?? Array.Empty<ChatThreadSnapshot>();
    public CurrentChatSnapshot? CurrentChat => _state.currentChat;
    public GroupDetailsSnapshot? GroupDetails => _state.groupDetails;
    public PublicInviteSnapshot? PublicInvite => _state.publicInvite;
    public LinkDeviceSnapshot? LinkDevice => _state.linkDevice;
    public NetworkStatusSnapshot? NetworkStatus => _state.networkStatus;
    public PreferencesSnapshot Preferences => _state.preferences;
    public BusyState Busy => _state.busy;
    public DesktopNearbySnapshot NearbySnapshot => _nearbySnapshot;

    public HashtreeAttachmentCache AttachmentCache => _cache;
    public bool UpdateChecking
    {
        get => _updateChecking;
        private set
        {
            if (SetField(ref _updateChecking, value))
            {
                Notify(nameof(UpdateInstallEnabled));
            }
        }
    }

    public bool UpdateInstalling
    {
        get => _updateInstalling;
        private set
        {
            if (SetField(ref _updateInstalling, value))
            {
                Notify(nameof(UpdateInstallEnabled));
            }
        }
    }

    public bool UpdateAvailable
    {
        get => _updateAvailable;
        private set
        {
            if (SetField(ref _updateAvailable, value))
            {
                Notify(nameof(UpdateInstallEnabled));
                Notify(nameof(UpdateStripeText));
            }
        }
    }

    public string UpdateVersion
    {
        get => _updateVersion;
        private set
        {
            if (SetField(ref _updateVersion, value))
            {
                Notify(nameof(UpdateStripeText));
            }
        }
    }

    public string UpdateStatus
    {
        get => _updateStatus;
        private set => SetField(ref _updateStatus, value);
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
            if (value && UpdateInstallEnabled)
            {
                _ = InstallUpdateAsync();
            }
        }
    }

    public bool UpdateInstallEnabled => UpdateAvailable && _updateHasAsset && !UpdateChecking && !UpdateInstalling;

    public string UpdateStripeText => string.IsNullOrWhiteSpace(UpdateVersion)
        ? "Update available"
        : $"{UpdateVersion} available";

    // ───────────────────────────── navigation ─────────────────────────────────

    public void NavigateBack()
    {
        var stack = _state.router.screenStack ?? Array.Empty<Screen>();
        if (stack.Length == 0) return;
        var next = stack.Take(stack.Length - 1).ToArray();
        NavigateOptimistically(next, new AppAction.UpdateScreenStack(next), showToastOnFailure: false);
    }

    public void Push(Screen screen)
    {
        var stack = StackByApplyingPushScreen(screen);
        if (stack is null)
        {
            DispatchToRust(new AppAction.PushScreen(screen));
            return;
        }
        NavigateOptimistically(stack, new AppAction.PushScreen(screen));
    }

    public ChatThreadSnapshot[] MutualGroups(string ownerInput)
    {
        try
        {
            return _ffi.MutualGroups(ownerInput.Trim()).groups ?? Array.Empty<ChatThreadSnapshot>();
        }
        catch (Exception error)
        {
            LogFfiFailure("ffiapp.mutual_groups", error);
            return Array.Empty<ChatThreadSnapshot>();
        }
    }

    // ───────────────────────────── account ────────────────────────────────────

    public void CreateAccount(string name)
    {
        var t = name.Trim();
        if (string.IsNullOrEmpty(t)) return;
        DispatchToRust(new AppAction.CreateAccount(t));
    }

    public void RestoreSession(string ownerNsec)
    {
        var t = ownerNsec.Trim();
        if (string.IsNullOrEmpty(t)) return;
        DispatchToRust(new AppAction.RestoreSession(t));
    }

    public void UpdateProfileMetadata(string name, string? pictureUrl, string? about)
    {
        var t = name.Trim();
        if (string.IsNullOrEmpty(t)) return;
        var p = pictureUrl?.Trim();
        var a = about?.Trim();
        DispatchToRust(new AppAction.UpdateProfileMetadata(
            t,
            string.IsNullOrEmpty(p) ? null : p,
            string.IsNullOrEmpty(a) ? null : a));
    }

    public void UploadProfilePicture(string sourceFilePath)
    {
        try
        {
            var staged = _cache.StageOutgoing(sourceFilePath);
            DispatchToRust(new AppAction.UploadProfilePicture(staged.Path));
        }
        catch
        {
            ShowToast("Image could not be opened");
        }
    }

    public void Logout()
    {
        _automaticRevocationLogoutInFlight = true;
        if (!_secretStore.Clear() || !_pendingDeviceLinkSecretStore.Clear())
        {
            _automaticRevocationLogoutInFlight = false;
            ShowToast("Could not clear secret key.");
            return;
        }
        DispatchToRust(new AppAction.Logout());
        try { Directory.Delete(_dataDir, recursive: true); } catch { }
        Directory.CreateDirectory(_dataDir);
    }

    public void Shutdown()
    {
        StopDesktopUpdateChecks();
        try { _ffi.Shutdown(); } catch { }
    }

    public void AppForegrounded()
    {
        DispatchToRust(new AppAction.AppForegrounded(), showToastOnFailure: false);
        StartDesktopUpdateChecks();
    }

    public void AppWindowActivated()
    {
        RecordUserActivity();
        AppForegrounded();
        Notify(nameof(CanMarkActiveChatSeen));
    }

    public void AppWindowDeactivated() =>
        Notify(nameof(CanMarkActiveChatSeen));

    public void RecordUserActivity()
    {
        var now = DateTimeOffset.UtcNow;
        if (now - _lastUserActivityUtc < TimeSpan.FromMilliseconds(250)) return;
        var wasEligible = CanMarkActiveChatSeen;
        _lastUserActivityUtc = now;
        if (!wasEligible)
        {
            Notify(nameof(CanMarkActiveChatSeen));
        }
    }

    public bool CanMarkActiveChatSeen
    {
        get
        {
            var mainWindow = Application.Current?.MainWindow;
            return mainWindow?.IsActive == true &&
                   DateTimeOffset.UtcNow - _lastUserActivityUtc <= ActiveChatSeenIdleLimit;
        }
    }

    // ───────────────────────── linked devices ─────────────────────────────────

    public void StartLinkedDevice(string ownerInput)
    {
        DispatchToRust(new AppAction.StartLinkedDevice(ownerInput.Trim()));
    }

    public void AddAuthorizedDevice(string deviceInput)
    {
        var t = deviceInput.Trim();
        if (string.IsNullOrEmpty(t)) return;
        DispatchToRust(new AppAction.AddAuthorizedDevice(t));
    }

    public void RemoveAuthorizedDevice(string devicePubkeyHex)
    {
        var t = devicePubkeyHex.Trim();
        if (string.IsNullOrEmpty(t)) return;
        DispatchToRust(new AppAction.RemoveAuthorizedDevice(t));
    }

    public void AcknowledgeRevokedDevice() =>
        DispatchToRust(new AppAction.AcknowledgeRevokedDevice());

    // ───────────────────────────── chats ──────────────────────────────────────

    public void CreateChat(string peerInput)
    {
        var t = peerInput.Trim();
        if (string.IsNullOrEmpty(t)) return;
        DispatchToRust(new AppAction.CreateChat(t));
    }

    public void OpenChat(string chatId)
    {
        var trimmed = chatId.Trim();
        if (string.IsNullOrEmpty(trimmed)) return;
        NavigateOptimistically(
            new Screen[] { new Screen.Chat(trimmed) },
            new AppAction.OpenChat(trimmed)
        );
    }

    public void SendMessage(string chatId, string text)
    {
        var c = chatId.Trim();
        var t = text.Trim();
        if (string.IsNullOrEmpty(c) || string.IsNullOrEmpty(t)) return;
        DispatchToRust(new AppAction.SendMessage(c, t));
    }

    public void SendDisappearing(string chatId, string text, ulong expiresAtSecs) =>
        DispatchToRust(new AppAction.SendDisappearingMessage(chatId, text, expiresAtSecs));

    public void SetChatMessageTtl(string chatId, ulong? ttlSeconds) =>
        DispatchToRust(new AppAction.SetChatMessageTtl(chatId, ttlSeconds));

    public void SetChatMuted(string chatId, bool muted) =>
        DispatchToRust(new AppAction.SetChatMuted(chatId, muted));

    public void SetContactNickname(string ownerPubkeyHex, string nickname) =>
        DispatchToRust(new AppAction.SetContactNickname(ownerPubkeyHex.Trim(), nickname.Trim()));

    public void SetChatPinned(string chatId, bool pinned) =>
        DispatchToRust(new AppAction.SetChatPinned(chatId, pinned));

    public void SetChatUnread(string chatId, bool unread) =>
        DispatchToRust(new AppAction.SetChatUnread(chatId, unread));

    public void ToggleReaction(string chatId, string messageId, string emoji) =>
        DispatchToRust(new AppAction.ToggleReaction(chatId, messageId, emoji));

    public void DeleteLocalMessage(string chatId, string messageId) =>
        DispatchToRust(new AppAction.DeleteLocalMessage(chatId, messageId));

    public void MarkMessagesSeen(string chatId, string[] messageIds) =>
        DispatchToRust(new AppAction.MarkMessagesSeen(chatId, messageIds));

    public void SendTyping(string chatId) =>
        DispatchToRust(new AppAction.SendTyping(chatId));

    public void StopTyping(string chatId) =>
        DispatchToRust(new AppAction.StopTyping(chatId));

    public void SendAttachments(string chatId, IList<string> sourceFilePaths, string caption)
    {
        var c = chatId.Trim();
        if (string.IsNullOrEmpty(c) || sourceFilePaths == null || sourceFilePaths.Count == 0) return;
        try
        {
            var staged = sourceFilePaths
                .Select(p => _cache.StageOutgoing(p))
                .Select(s => new OutgoingAttachment(s.Path, s.Filename))
                .ToArray();
            DispatchToRust(new AppAction.SendAttachments(c, staged, caption?.Trim() ?? string.Empty));
        }
        catch
        {
            ShowToast("Attachment could not be opened");
        }
    }

    public Task<byte[]?> DownloadAttachmentAsync(MessageAttachmentSnapshot attachment) =>
        _cache.ResolveAttachmentAsync(attachment);

    public async Task<bool> OpenAttachmentAsync(MessageAttachmentSnapshot attachment)
    {
        var data = await DownloadAttachmentAsync(attachment).ConfigureAwait(false);
        if (data == null) { ShowToast("Attachment could not be opened"); return false; }
        try
        {
            var path = _cache.GetCachedAttachmentPath(attachment, data);
            if (!PlatformDocumentOpener.Open(path))
            {
                ShowToast("Attachment could not be opened");
                return false;
            }
            return true;
        }
        catch
        {
            ShowToast("Attachment could not be opened");
            return false;
        }
    }

    public Task<byte[]?> ResolveProfilePictureAsync(string nhash) =>
        _cache.ResolvePictureAsync(nhash);

    // ─────────────────────────── invites / groups ─────────────────────────────

    public void CreatePublicInvite() =>
        DispatchToRust(new AppAction.CreatePublicInvite());

    public void AcceptInvite(string inviteInput)
    {
        var t = inviteInput.Trim();
        if (string.IsNullOrEmpty(t)) return;
        DispatchToRust(new AppAction.AcceptInvite(t));
    }

    public void CreateGroup(string name, IList<string> memberInputs, string? sourcePicturePath = null)
    {
        var n = name.Trim();
        if (string.IsNullOrEmpty(n) || memberInputs == null || memberInputs.Count == 0) return;
        var members = memberInputs.Select(s => s.Trim()).Where(s => s.Length > 0).ToArray();
        if (!string.IsNullOrWhiteSpace(sourcePicturePath))
        {
            try
            {
                var staged = _cache.StageOutgoing(sourcePicturePath);
                DispatchToRust(new AppAction.CreateGroupWithPicture(n, members, staged.Path, staged.Filename));
                return;
            }
            catch
            {
                ShowToast("Image could not be opened");
            }
        }
        DispatchToRust(new AppAction.CreateGroup(n, members));
    }

    public void UpdateGroupName(string groupId, string name) =>
        DispatchToRust(new AppAction.UpdateGroupName(groupId, name.Trim()));

    public void UpdateGroupAbout(string groupId, string? about)
    {
        var normalized = about?.Trim();
        DispatchToRust(new AppAction.UpdateGroupAbout(
            groupId,
            string.IsNullOrEmpty(normalized) ? null : normalized
        ));
    }

    public void UpdateGroupPicture(string groupId, string sourceFilePath)
    {
        try
        {
            var staged = _cache.StageOutgoing(sourceFilePath);
            DispatchToRust(new AppAction.UpdateGroupPicture(groupId, staged.Path, staged.Filename));
        }
        catch
        {
            ShowToast("Image could not be opened");
        }
    }

    public void AddGroupMembers(string groupId, IList<string> memberInputs) =>
        DispatchToRust(new AppAction.AddGroupMembers(
            groupId,
            memberInputs.Select(s => s.Trim()).Where(s => s.Length > 0).ToArray()
        ));

    public void SetGroupAdmin(string groupId, string ownerPubkeyHex, bool isAdmin) =>
        DispatchToRust(new AppAction.SetGroupAdmin(groupId, ownerPubkeyHex, isAdmin));

    public void RemoveGroupMember(string groupId, string ownerPubkeyHex) =>
        DispatchToRust(new AppAction.RemoveGroupMember(groupId, ownerPubkeyHex));

    public void DeleteChat(string chatId) =>
        DispatchToRust(new AppAction.DeleteChat(chatId));

    // ─────────────────────────────── settings ─────────────────────────────────

    public void SetTypingIndicatorsEnabled(bool enabled) =>
        DispatchToRust(new AppAction.SetTypingIndicatorsEnabled(enabled));

    public void SetReadReceiptsEnabled(bool enabled) =>
        DispatchToRust(new AppAction.SetReadReceiptsEnabled(enabled));

    public void SetAcceptUnknownDirectMessages(bool enabled) =>
        DispatchToRust(new AppAction.SetAcceptUnknownDirectMessages(enabled));

    public void SetDesktopNotificationsEnabled(bool enabled) =>
        DispatchToRust(new AppAction.SetDesktopNotificationsEnabled(enabled));

    public void SetStartupAtLoginEnabled(bool enabled)
    {
        try
        {
            PlatformStartupAtLogin.SetEnabled(enabled);
            DispatchToRust(new AppAction.SetStartupAtLoginEnabled(enabled));
        }
        catch
        {
            ShowToast("Startup setting unavailable");
        }
    }

    private void SyncStartupAtLoginPreference()
    {
        if (!PlatformStartupAtLogin.IsSupported) return;
        try { PlatformStartupAtLogin.SetEnabled(_state.preferences.startupAtLoginEnabled); }
        catch { }
    }

    public void SetNearbyLanEnabled(bool enabled)
    {
        if (enabled)
        {
            if (_state.preferences.nearbyEnabled)
                StartNearbySafely(showToastOnFailure: true);
        }
        else
        {
            StopNearbySafely(showToastOnFailure: true);
        }

        DispatchToRust(new AppAction.SetNearbyLanEnabled(enabled));
    }

    public void SetNearbyEnabled(bool enabled)
    {
        if (!enabled)
            StopNearbySafely(showToastOnFailure: false);

        DispatchToRust(new AppAction.SetNearbyEnabled(enabled));
    }

    public void SetNearbyMailbagEnabled(bool enabled) =>
        DispatchToRust(new AppAction.SetNearbyMailbagEnabled(enabled));

    public void SetNearbyShowInChatList(bool enabled) =>
        DispatchToRust(new AppAction.SetNearbyShowInChatList(enabled));

    public void AddNostrRelay(string url) =>
        DispatchToRust(new AppAction.AddNostrRelay(url.Trim()));

    public void UpdateNostrRelay(string oldUrl, string newUrl) =>
        DispatchToRust(new AppAction.UpdateNostrRelay(oldUrl.Trim(), newUrl.Trim()));

    public void RemoveNostrRelay(string url) =>
        DispatchToRust(new AppAction.RemoveNostrRelay(url.Trim()));

    public void ResetNostrRelays() =>
        DispatchToRust(new AppAction.ResetNostrRelays());

    public void SetImageProxyEnabled(bool enabled) =>
        DispatchToRust(new AppAction.SetImageProxyEnabled(enabled));

    public void SetImageProxyUrl(string url) =>
        DispatchToRust(new AppAction.SetImageProxyUrl(url.Trim()));

    public void SetImageProxyKeyHex(string keyHex) =>
        DispatchToRust(new AppAction.SetImageProxyKeyHex(keyHex.Trim()));

    public void SetImageProxySaltHex(string saltHex) =>
        DispatchToRust(new AppAction.SetImageProxySaltHex(saltHex.Trim()));

    public void ResetImageProxySettings() =>
        DispatchToRust(new AppAction.ResetImageProxySettings());

    // ───────────────────────────── updates ───────────────────────────────────

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
            _ = CheckForUpdatesAsync(manual: false);
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
            _ = CheckForUpdatesAsync(manual: false);
        };
        _updatePollTimer.Start();
    }

    private void StopDesktopUpdateChecks()
    {
        _updatePollTimer?.Stop();
        _updatePollTimer = null;
    }

    public async Task CheckForUpdatesAsync(bool manual = true)
    {
        if (UpdateChecking) return;
        UpdateChecking = true;
        if (manual)
        {
            UpdateStatus = "Checking for updates";
        }

        try
        {
            var result = await _updateService.CheckAsync().ConfigureAwait(true);
            _updateHasAsset = result.Available && result.HasAsset;
            UpdateAvailable = result.Available;
            UpdateVersion = result.Tag;
            Notify(nameof(UpdateInstallEnabled));

            if (result.Available)
            {
                UpdateStatus = result.Message;
                if (AutoInstallUpdates && UpdateInstallEnabled)
                {
                    await InstallUpdateAsync().ConfigureAwait(true);
                }
            }
            else if (manual)
            {
                UpdateStatus = result.Message;
            }
            else
            {
                UpdateStatus = "";
            }
        }
        catch (Exception error)
        {
            if (manual)
            {
                UpdateStatus = error.Message;
            }
        }
        finally
        {
            UpdateChecking = false;
        }
    }

    public async Task InstallUpdateAsync()
    {
        if (!_updateHasAsset || UpdateInstalling) return;
        UpdateInstalling = true;
        UpdateStatus = $"Downloading {UpdateVersion}";
        try
        {
            var path = await _updateService.DownloadWithEmbeddedUpdaterAsync().ConfigureAwait(true);
            UpdateStatus = $"Downloaded {Path.GetFileName(path)}";
            if (!UpdateService.SkipOpen)
            {
                _ = Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
            }
        }
        catch (Exception error)
        {
            UpdateStatus = error.Message;
        }
        finally
        {
            UpdateInstalling = false;
        }
    }

    // ────────────────────── support / build metadata ─────────────────────────

    public string SupportBundleJson() => SafeSupportBundleJson();
    public string BuildSummary() => Native.BuildSummary();
    public string AppVersion() => BuildSummary().Split(' ', StringSplitOptions.RemoveEmptyEntries).FirstOrDefault() ?? "";
    public string RelaySetIdText() => Native.RelaySetId();
    public bool TrustedTestBuildEnabled() => Native.IsTrustedTestBuild();
    public string? ExportOwnerNsec() => _secretStore.Load()?.OwnerNsec;

    private static TimeSpan LoadDesktopUpdatePollInterval()
    {
        var raw = Environment.GetEnvironmentVariable("IRIS_UPDATE_POLL_SECONDS");
        return double.TryParse(raw, out var seconds) && seconds > 0
            ? TimeSpan.FromSeconds(seconds)
            : TimeSpan.FromHours(6);
    }

    public void CopyToClipboard(string value)
    {
        PlatformClipboard.SetString(value);
        ShowToast("Copied");
    }

    // ───────────────────────────── plumbing ───────────────────────────────────

    private void TryRestorePersistedSession()
    {
        var pendingLink = _pendingDeviceLinkSecretStore.LoadPendingDeviceLink();
        if (pendingLink != null)
        {
            _persistedRestoreInFlight = true;
            BootstrapInFlight = true;
            Notify(nameof(BootstrapInFlight));
            var restored = DispatchToRust(new AppAction.RestorePendingDeviceLink(
                pendingLink.DeviceNsec,
                pendingLink.ApprovalBootstrapJson
            ), showToastOnFailure: false);
            if (!restored)
            {
                _persistedRestoreInFlight = false;
                BootstrapInFlight = false;
                Notify(nameof(BootstrapInFlight));
            }
            return;
        }
        var bundle = _secretStore.Load();
        if (bundle == null)
        {
            BootstrapInFlight = false;
            Notify(nameof(BootstrapInFlight));
            return;
        }
        _persistedRestoreInFlight = true;
        BootstrapInFlight = true;
        Notify(nameof(BootstrapInFlight));
        var dispatched = DispatchToRust(new AppAction.RestoreAccountBundle(
            bundle.OwnerNsec,
            bundle.OwnerPubkeyHex,
            bundle.DeviceNsec
        ), showToastOnFailure: false);
        if (!dispatched)
        {
            _persistedRestoreInFlight = false;
            BootstrapInFlight = false;
            Notify(nameof(BootstrapInFlight));
        }
    }

    private void Apply(AppUpdate update)
    {
        switch (update)
        {
            case AppUpdate.PersistAccountBundle p:
                _secretStore.Save(new WindowsCredentialStore.StoredAccountBundle(
                    p.ownerNsec,
                    p.ownerPubkeyHex,
                    p.deviceNsec
                ));
                _pendingDeviceLinkSecretStore.Clear();
                break;

            case AppUpdate.PersistPendingDeviceLink p:
                _pendingDeviceLinkSecretStore.SavePendingDeviceLink(
                    new WindowsCredentialStore.StoredPendingDeviceLink(
                        p.deviceNsec,
                        p.approvalBootstrapJson
                    )
                );
                break;

            case AppUpdate.ClearPendingDeviceLink:
                _pendingDeviceLinkSecretStore.Clear();
                break;

            case AppUpdate.FullState f:
                if (f.v1.rev <= _lastRevApplied) return;
                var prev = _state;
                var next = StateByReconcilingPendingNavigation(f.v1);
                _state = next;
                _lastRevApplied = f.v1.rev;
                if (next.account?.authorizationState != DeviceAuthorizationState.Revoked)
                {
                    _automaticRevocationLogoutInFlight = false;
                }
                if (LogoutIfCurrentDeviceRevoked(next))
                {
                    NotifyAll();
                    break;
                }
                SettleBootstrapIfNeeded(next);
                SyncCurrentDeviceLabels(next);
                SyncNearbyPreference(prev, next);
                PostDesktopNotifications(prev, next);
                NotifyAll();
                if (!string.IsNullOrEmpty(next.toast))
                {
                    ShowToast(next.toast!);
                }
                break;

            case AppUpdate.NearbyPublishedEvent nearby:
                _ = Task.Run(() => PublishNearbySafely(nearby));
                break;
            case AppUpdate.NearbyPeersChanged nearby:
                ApplyNearbySnapshot(nearby.snapshot);
                break;
        }
    }

    private bool LogoutIfCurrentDeviceRevoked(AppState next)
    {
        if (next.account?.authorizationState != DeviceAuthorizationState.Revoked ||
            _automaticRevocationLogoutInFlight)
        {
            return false;
        }

        Logout();
        return true;
    }

    private void SettleBootstrapIfNeeded(AppState next)
    {
        if (!_persistedRestoreInFlight)
        {
            BootstrapInFlight = false;
            return;
        }
        if (next.account == null && next.busy.restoringSession)
        {
            BootstrapInFlight = true;
            return;
        }
        _persistedRestoreInFlight = false;
        BootstrapInFlight = false;
    }

    private void SyncCurrentDeviceLabels(AppState state)
    {
        if (state.account == null)
        {
            _lastSyncedDeviceLabelsKey = null;
            return;
        }

        var currentDevice = state.deviceRoster?.devices.FirstOrDefault(d => d.isCurrentDevice);
        var deviceLabel = string.IsNullOrWhiteSpace(currentDevice?.deviceLabel)
            ? PlatformDeviceLabels.CurrentDeviceLabel
            : currentDevice!.deviceLabel!.Trim();
        var clientLabel = string.IsNullOrWhiteSpace(currentDevice?.clientLabel)
            ? PlatformDeviceLabels.CurrentClientLabel
            : currentDevice!.clientLabel!.Trim();
        var key = $"{deviceLabel}\u001F{clientLabel}";
        if (key == _lastSyncedDeviceLabelsKey) return;
        _lastSyncedDeviceLabelsKey = key;

        DispatchToRust(
            new AppAction.SetCurrentDeviceLabels(deviceLabel, clientLabel),
            showToastOnFailure: false
        );
    }

    private void ApplyNearbySnapshot(DesktopNearbySnapshot snapshot)
    {
        _nearbySnapshot = snapshot;
        Notify(nameof(NearbySnapshot));
    }

    private void SyncNearbyPreference(AppState old, AppState next)
    {
        var wasVisible = old.preferences.nearbyEnabled && old.preferences.nearbyLanEnabled;
        var shouldBeVisible = next.preferences.nearbyEnabled && next.preferences.nearbyLanEnabled;
        if (shouldBeVisible && !NearbySnapshot.visible)
            StartNearbySafely(showToastOnFailure: false);
        else if (!shouldBeVisible && (wasVisible || NearbySnapshot.visible))
            StopNearbySafely(showToastOnFailure: false);
    }

    private Screen[]? StackByApplyingPushScreen(Screen screen)
    {
        if (_state.account == null)
        {
            return screen switch
            {
                Screen.Welcome => Array.Empty<Screen>(),
                Screen.CreateAccount or Screen.RestoreAccount or Screen.AddDevice => new[] { screen },
                _ => null,
            };
        }

        switch (screen)
        {
            case Screen.ChatList:
                return Array.Empty<Screen>();
            case Screen.NewChat:
            case Screen.NewGroup:
            case Screen.CreateInvite:
            case Screen.JoinInvite:
            case Screen.Settings:
            case Screen.DeviceRoster:
                return new[] { screen };
            case Screen.Chat chat:
            {
                var trimmed = chat.chatId.Trim();
                return string.IsNullOrEmpty(trimmed) ? null : new Screen[] { new Screen.Chat(trimmed) };
            }
            case Screen.DirectChatInfo info:
            {
                var trimmed = info.chatId.Trim();
                if (string.IsNullOrEmpty(trimmed)) return null;
                var infoScreen = new Screen.DirectChatInfo(trimmed);
                var stack = _state.router.screenStack ?? Array.Empty<Screen>();
                return stack.LastOrDefault()?.Equals(infoScreen) == true
                    ? stack
                    : stack.Append(infoScreen).ToArray();
            }
            case Screen.GroupDetails details:
            {
                var groupId = details.groupId.Trim();
                if (string.IsNullOrEmpty(groupId)) return null;
                var groupChatId = $"group:{groupId}";
                var stack = ActiveChatId(_state) == groupChatId
                    ? (_state.router.screenStack ?? Array.Empty<Screen>())
                    : new Screen[] { new Screen.Chat(groupChatId) };
                var detailsScreen = new Screen.GroupDetails(groupId);
                return stack.LastOrDefault()?.Equals(detailsScreen) == true
                    ? stack
                    : stack.Append(detailsScreen).ToArray();
            }
            default:
                return null;
        }
    }

    private bool NavigateOptimistically(Screen[] stack, AppAction action, bool showToastOnFailure = true)
    {
        _pendingNavigationOverride = new PendingNavigationOverride(
            stack,
            DateTimeOffset.UtcNow.Add(NavigationOverrideTtl)
        );
        ApplyLocalScreenStack(stack);
        var dispatched = DispatchToRust(
            action,
            showToastOnFailure: showToastOnFailure,
            preservesPendingNavigation: true
        );
        if (!dispatched)
        {
            _pendingNavigationOverride = null;
        }
        return dispatched;
    }

    private AppState StateByReconcilingPendingNavigation(AppState next)
    {
        var pending = _pendingNavigationOverride;
        if (pending == null) return next;
        if (next.account == null)
        {
            _pendingNavigationOverride = null;
            return next;
        }
        if (next.router.screenStack.SequenceEqual(pending.Stack))
        {
            _pendingNavigationOverride = null;
            return next;
        }
        if (DateTimeOffset.UtcNow >= pending.ExpiresAt)
        {
            _pendingNavigationOverride = null;
            return next;
        }
        return StateByApplyingLocalScreenStack(pending.Stack, next);
    }

    private void ApplyLocalScreenStack(Screen[] stack)
    {
        _state = StateByApplyingLocalScreenStack(stack, _state);
        NotifyAll();
    }

    private AppState StateByApplyingLocalScreenStack(Screen[] stack, AppState baseState)
    {
        var active = stack.LastOrDefault() ?? baseState.router.defaultScreen;
        var currentChat = baseState.currentChat;
        var groupDetails = baseState.groupDetails;
        switch (active)
        {
            case Screen.Chat chat:
                if (currentChat?.chatId != chat.chatId)
                {
                    currentChat = SafeChatSnapshot(chat.chatId, RouteChatSnapshotLimit);
                }
                groupDetails = null;
                break;
            case Screen.DirectChatInfo info:
                if (currentChat?.chatId != info.chatId)
                {
                    currentChat = SafeChatSnapshot(info.chatId, RouteChatSnapshotLimit);
                }
                groupDetails = null;
                break;
            case Screen.GroupDetails details:
                if (groupDetails?.groupId != details.groupId) groupDetails = null;
                break;
            default:
                currentChat = null;
                groupDetails = null;
                break;
        }
        return baseState with
        {
            router = new Router(baseState.router.defaultScreen, stack),
            currentChat = currentChat,
            groupDetails = groupDetails
        };
    }

    private static string? ActiveChatId(AppState state)
    {
        var active = state.router.screenStack.LastOrDefault() ?? state.router.defaultScreen;
        if (active is Screen.Chat chat)
        {
            return chat.chatId.Trim();
        }
        return state.currentChat?.chatId?.Trim();
    }

    private void ApplySafely(AppUpdate update)
    {
        try
        {
            Apply(update);
        }
        catch (Exception error)
        {
            LogFfiFailure("ffi.update_callback.failed", error, update.GetType().Name);
            ShowToast(DispatchFailureToast);
        }
    }

    private AppState SafeState()
    {
        try
        {
            return _ffi.State();
        }
        catch (Exception error)
        {
            LogFfiFailure("ffiapp.state", error);
            return FallbackState(CoreRestartToast);
        }
    }

    private FfiDesktopNearby? CreateNearbySafely()
    {
        try
        {
            return new FfiDesktopNearby(_ffi, new NearbyObserver(this));
        }
        catch (Exception error)
        {
            LogFfiFailure("desktop_nearby.new", error);
            return null;
        }
    }

    private DesktopNearbySnapshot SafeNearbySnapshot()
    {
        if (_nearby is null) return EmptyNearbySnapshot();
        try
        {
            return _nearby.Snapshot();
        }
        catch (Exception error)
        {
            LogFfiFailure("desktop_nearby.snapshot", error);
            return EmptyNearbySnapshot();
        }
    }

    private void ListenForUpdatesSafely()
    {
        try
        {
            _ffi.ListenForUpdates(new Reconciler(this));
        }
        catch (Exception error)
        {
            LogFfiFailure("ffiapp.listen_for_updates", error);
            ShowToast(CoreRestartToast);
        }
    }

    private CurrentChatSnapshot? SafeChatSnapshot(string chatId, uint limit)
    {
        try
        {
            return _ffi.ChatSnapshot(chatId, limit);
        }
        catch (Exception error)
        {
            LogFfiFailure("ffiapp.chat_snapshot", error);
            ShowToast(DispatchFailureToast);
            return null;
        }
    }

    private string SafeSupportBundleJson()
    {
        string rustJson;
        try
        {
            rustJson = _ffi.ExportSupportBundleJson();
        }
        catch (Exception error)
        {
            LogFfiFailure("ffiapp.export_support_bundle_json", error);
            rustJson = "{}";
        }
        return SupportBundleJsonWithClientLog(rustJson);
    }

    private bool StartNearbySafely(bool showToastOnFailure)
    {
        if (_nearby is null)
        {
            if (showToastOnFailure) ShowToast(DispatchFailureToast);
            return false;
        }
        try
        {
            _nearby.Start(LocalDeviceName());
            return true;
        }
        catch (Exception error)
        {
            LogFfiFailure("desktop_nearby.start", error);
            if (showToastOnFailure) ShowToast(DispatchFailureToast);
            return false;
        }
    }

    private bool StopNearbySafely(bool showToastOnFailure)
    {
        if (_nearby is null) return true;
        try
        {
            _nearby.Stop();
            return true;
        }
        catch (Exception error)
        {
            LogFfiFailure("desktop_nearby.stop", error);
            if (showToastOnFailure) ShowToast(DispatchFailureToast);
            return false;
        }
    }

    private bool PublishNearbySafely(AppUpdate.NearbyPublishedEvent nearby)
    {
        if (_nearby is null) return false;
        try
        {
            _nearby.Publish(
                nearby.eventId,
                nearby.kind,
                nearby.createdAtSecs,
                nearby.eventJson
            );
            return true;
        }
        catch (Exception error)
        {
            LogFfiFailure("desktop_nearby.publish", error);
            return false;
        }
    }

    private void PostDesktopNotifications(AppState old, AppState next)
    {
        if (old.account == null) return;

        var mainWindow = Application.Current?.MainWindow;
        var appForeground = mainWindow?.IsActive ?? false;
        var openChatId = Native.RouterOpenChatId(next.router);

        var candidates = Native.DecidePendingNotifications(
            previousChats: old.chatList ?? Array.Empty<ChatThreadSnapshot>(),
            nextChats: next.chatList ?? Array.Empty<ChatThreadSnapshot>(),
            preferences: next.preferences,
            appForeground: appForeground,
            openChatId: openChatId
        );
        foreach (var candidate in candidates)
        {
            try { _notifier.Post(candidate.title, candidate.body); } catch { }
        }
    }

    private void NotifyAll()
    {
        Notify(nameof(State));
        Notify(nameof(BootstrapInFlight));
        Notify(nameof(ActiveScreen));
        Notify(nameof(CanNavigateBack));
        Notify(nameof(Account));
        Notify(nameof(DeviceRoster));
        Notify(nameof(ChatList));
        Notify(nameof(CurrentChat));
        Notify(nameof(GroupDetails));
        Notify(nameof(PublicInvite));
        Notify(nameof(NetworkStatus));
        Notify(nameof(Preferences));
        Notify(nameof(Busy));
        Notify(nameof(NearbySnapshot));
    }

    private bool DispatchToRust(
        AppAction action,
        bool showToastOnFailure = true,
        bool preservesPendingNavigation = false
    )
    {
        if (!preservesPendingNavigation && ActionClearsPendingNavigation(action))
        {
            _pendingNavigationOverride = null;
        }
        try
        {
            _ffi.Dispatch(action);
            return true;
        }
        catch (Exception error)
        {
            LogFfiFailure("ffi.dispatch.failed", error, ActionLogName(action));
            if (showToastOnFailure)
            {
                ShowToast(DispatchFailureToast);
            }
            return false;
        }
    }

    private static bool ActionClearsPendingNavigation(AppAction action) =>
        action is AppAction.OpenChat ||
        action is AppAction.PushScreen ||
        action is AppAction.UpdateScreenStack ||
        action is AppAction.NavigateBack ||
        action is AppAction.CreateChat ||
        action is AppAction.CreateGroup ||
        action is AppAction.CreateGroupWithPicture ||
        action is AppAction.AcceptInvite ||
        action is AppAction.Logout ||
        action is AppAction.RestoreSession ||
        action is AppAction.RestoreAccountBundle ||
        action is AppAction.RestorePendingDeviceLink;

    private static string ActionLogName(AppAction action) =>
        action.GetType().Name;

    public void ShowToast(string text)
    {
        lock (_toastLock) _activeToast = text;
        Notify(nameof(ToastMessage));

        var captured = text;
        var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(3) };
        timer.Tick += (_, _) =>
        {
            timer.Stop();
            lock (_toastLock)
            {
                if (_activeToast == captured) _activeToast = null;
            }
            Notify(nameof(ToastMessage));
        };
        timer.Start();
    }

    private void Notify([CallerMemberName] string? name = null) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));

    private bool SetField<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value)) return false;
        field = value;
        Notify(name);
        return true;
    }

    private static AppState FallbackState(string? toast) => new(
        0,
        new Router(new Screen.Welcome(), Array.Empty<Screen>()),
        null,
        null,
        new BusyState(
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            null
        ),
        Array.Empty<ChatThreadSnapshot>(),
        null,
        null,
        null,
        null,
        null,
        new MobilePushSyncSnapshot(
            null,
            Array.Empty<string>(),
            Array.Empty<string>(),
            Array.Empty<MobilePushSessionSnapshot>()
        ),
        new PreferencesSnapshot(
            sendTypingIndicators: true,
            sendReadReceipts: true,
            desktopNotificationsEnabled: true,
            inviteAcceptanceNotificationsEnabled: true,
            startupAtLoginEnabled: true,
            nearbyEnabled: true,
            nearbyBluetoothEnabled: false,
            nearbyLanEnabled: false,
            nearbyShowInChatList: true,
            nearbyMailbagEnabled: true,
            nostrRelayUrls: new[]
            {
                "wss://relay.damus.io",
                "wss://nos.lol",
                "wss://relay.primal.net",
                "wss://relay.snort.social",
                "wss://temp.iris.to"
            },
            imageProxyEnabled: true,
            imageProxyUrl: "https://imgproxy.iris.to",
            imageProxyKeyHex: "f66233cb160ea07078ff28099bfa3e3e654bc10aa4a745e12176c433d79b8996",
            imageProxySaltHex: "5e608e60945dcd2a787e8465d76ba34149894765061d39287609fb9d776caa0c",
            mutedChatIds: Array.Empty<string>(),
            pinnedChatIds: Array.Empty<string>(),
            blockedOwnerPubkeys: Array.Empty<string>(),
            acceptedOwnerPubkeys: Array.Empty<string>(),
            debugLoggingEnabled: false,
            acceptUnknownDirectMessages: true,
            mobilePushServerUrl: ""
        ),
        toast
    );

    private static DesktopNearbySnapshot EmptyNearbySnapshot() =>
        new(false, "Off", Array.Empty<DesktopNearbyPeerSnapshot>());

    private void LogFfiFailure(string category, Exception error, string detail = "")
    {
        var logDetail = string.IsNullOrWhiteSpace(detail)
            ? ErrorSummary(error)
            : $"{detail}: {ErrorSummary(error)}";
        AppendClientDebugLog(category, logDetail);
        var message = $"Iris Chat FFI call failed ({category}): {logDetail}\n{error}";
        Trace.TraceError(message);
        Debug.WriteLine(message);
    }

    private void AppendClientDebugLog(string category, string detail)
    {
        var truncated = detail.Length <= MaxClientDebugLogDetailChars
            ? detail
            : detail[..MaxClientDebugLogDetailChars];
        lock (_clientDebugLogLock)
        {
            _clientDebugLog.Add(new ClientDebugLogEntry(
                DateTimeOffset.UtcNow.ToUnixTimeSeconds(),
                category,
                truncated
            ));
            if (_clientDebugLog.Count > MaxClientDebugLogEntries)
            {
                _clientDebugLog.RemoveRange(0, _clientDebugLog.Count - MaxClientDebugLogEntries);
            }
        }
    }

    private List<ClientDebugLogEntry> SnapshotClientDebugLog()
    {
        lock (_clientDebugLogLock)
        {
            return _clientDebugLog.ToList();
        }
    }

    private string SupportBundleJsonWithClientLog(string rustJson)
    {
        var clientLog = SnapshotClientDebugLog();
        if (clientLog.Count == 0) return rustJson;
        try
        {
            var root = JsonNode.Parse(string.IsNullOrWhiteSpace(rustJson) ? "{}" : rustJson) as JsonObject
                       ?? new JsonObject();
            var array = new JsonArray();
            foreach (var entry in clientLog)
            {
                array.Add(new JsonObject
                {
                    ["timestamp_secs"] = entry.TimestampSecs,
                    ["category"] = entry.Category,
                    ["detail"] = entry.Detail
                });
            }
            root["client_log"] = array;
            return root.ToJsonString(new JsonSerializerOptions { WriteIndented = true });
        }
        catch
        {
            return rustJson;
        }
    }

    private static string ErrorSummary(Exception error)
    {
        var message = error.Message;
        return string.IsNullOrWhiteSpace(message)
            ? error.GetType().Name
            : $"{error.GetType().Name}: {message}";
    }

    private sealed class Reconciler : AppReconciler
    {
        private readonly AppManager _owner;
        public Reconciler(AppManager owner) { _owner = owner; }

        public void Reconcile(AppUpdate update)
        {
            _owner._ui.BeginInvoke(new Action(() => _owner.ApplySafely(update)));
        }
    }

    private sealed class NearbyObserver : DesktopNearbyObserver
    {
        private readonly AppManager _owner;
        public NearbyObserver(AppManager owner) { _owner = owner; }

        public void DesktopNearbyChanged(DesktopNearbySnapshot snapshot)
        {
            _owner._ui.BeginInvoke(new Action(() => _owner.ApplyNearbySnapshot(snapshot)));
        }
    }

    private static string LocalDeviceName()
    {
        var name = Environment.MachineName?.Trim();
        return string.IsNullOrEmpty(name) ? "Iris" : name;
    }
}
