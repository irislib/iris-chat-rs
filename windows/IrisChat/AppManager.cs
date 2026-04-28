using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Threading;
using IrisChat.Bindings;

namespace IrisChat;

/// Windows shell-side AppManager. Mirrors the iOS/macOS AppManager.swift
/// contract: build the Rust core, restore secure credentials, render Rust
/// state, dispatch actions, persist secret side effects.
public sealed class AppManager : INotifyPropertyChanged
{
    public event PropertyChangedEventHandler? PropertyChanged;

    private readonly FfiApp _ffi;
    private readonly WindowsCredentialStore _secretStore;
    private readonly IDesktopNotificationPoster _notifier;
    private readonly HashtreeAttachmentCache _cache;
    private readonly string _dataDir;
    private readonly Dispatcher _ui;
    private readonly object _toastLock = new();
    private string? _activeToast;

    private AppState _state;
    private ulong _lastRevApplied;

    public AppManager(string dataDir, IDesktopNotificationPoster? notifier = null)
    {
        _dataDir = dataDir;
        _secretStore = new WindowsCredentialStore();
        _notifier = notifier ?? new SystemDesktopNotificationPoster();
        _cache = new HashtreeAttachmentCache(dataDir);
        _ui = Application.Current.Dispatcher;

        var version = typeof(AppManager).Assembly.GetName().Version?.ToString(3) ?? "0.1.0";
        _ffi = new FfiApp(dataDir, "", version);
        _state = _ffi.State();
        _lastRevApplied = _state.rev;

        _ffi.ListenForUpdates(new Reconciler(this));

        TryRestorePersistedSession();
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
    public NetworkStatusSnapshot? NetworkStatus => _state.networkStatus;
    public PreferencesSnapshot Preferences => _state.preferences;
    public BusyState Busy => _state.busy;

    public HashtreeAttachmentCache AttachmentCache => _cache;

    // ───────────────────────────── navigation ─────────────────────────────────

    public void NavigateBack()
    {
        var stack = _state.router.screenStack ?? Array.Empty<Screen>();
        if (stack.Length == 0) return;
        var next = stack.Take(stack.Length - 1).ToArray();
        _ffi.Dispatch(new AppAction.UpdateScreenStack(next));
    }

    public void Push(Screen screen) =>
        _ffi.Dispatch(new AppAction.PushScreen(screen));

    // ───────────────────────────── account ────────────────────────────────────

    public void CreateAccount(string name)
    {
        var t = name.Trim();
        if (string.IsNullOrEmpty(t)) return;
        _ffi.Dispatch(new AppAction.CreateAccount(t));
    }

    public void RestoreSession(string ownerNsec)
    {
        var t = ownerNsec.Trim();
        if (string.IsNullOrEmpty(t)) return;
        _ffi.Dispatch(new AppAction.RestoreSession(t));
    }

    public void UpdateProfileMetadata(string name, string? pictureUrl)
    {
        var t = name.Trim();
        if (string.IsNullOrEmpty(t)) return;
        var p = pictureUrl?.Trim();
        _ffi.Dispatch(new AppAction.UpdateProfileMetadata(t, string.IsNullOrEmpty(p) ? null : p));
    }

    public void UploadProfilePicture(string sourceFilePath)
    {
        try
        {
            var staged = _cache.StageOutgoing(sourceFilePath);
            _ffi.Dispatch(new AppAction.UploadProfilePicture(staged.Path));
        }
        catch
        {
            ShowToast("Image could not be opened");
        }
    }

    public void Logout()
    {
        _ffi.Dispatch(new AppAction.Logout());
        _secretStore.Clear();
        try { Directory.Delete(_dataDir, recursive: true); } catch { }
        Directory.CreateDirectory(_dataDir);
    }

    public void Shutdown()
    {
        try { _ffi.Shutdown(); } catch { }
    }

    public void AppForegrounded() => _ffi.Dispatch(new AppAction.AppForegrounded());

    // ───────────────────────── linked devices ─────────────────────────────────

    public void StartLinkedDevice(string ownerInput)
    {
        var normalized = Native.NormalizePeerInput(ownerInput.Trim());
        if (string.IsNullOrEmpty(normalized) || !Native.IsValidPeerInput(normalized)) return;
        _ffi.Dispatch(new AppAction.StartLinkedDevice(normalized));
    }

    public void AddAuthorizedDevice(string deviceInput)
    {
        var t = deviceInput.Trim();
        if (string.IsNullOrEmpty(t)) return;
        _ffi.Dispatch(new AppAction.AddAuthorizedDevice(t));
    }

    public void RemoveAuthorizedDevice(string devicePubkeyHex)
    {
        var t = devicePubkeyHex.Trim();
        if (string.IsNullOrEmpty(t)) return;
        _ffi.Dispatch(new AppAction.RemoveAuthorizedDevice(t));
    }

    public void AcknowledgeRevokedDevice() =>
        _ffi.Dispatch(new AppAction.AcknowledgeRevokedDevice());

    // ───────────────────────────── chats ──────────────────────────────────────

    public void CreateChat(string peerInput)
    {
        var t = peerInput.Trim();
        if (string.IsNullOrEmpty(t)) return;
        _ffi.Dispatch(new AppAction.CreateChat(t));
    }

    public void OpenChat(string chatId) =>
        _ffi.Dispatch(new AppAction.OpenChat(chatId));

    public void SendMessage(string chatId, string text)
    {
        var c = chatId.Trim();
        var t = text.Trim();
        if (string.IsNullOrEmpty(c) || string.IsNullOrEmpty(t)) return;
        _ffi.Dispatch(new AppAction.SendMessage(c, t));
    }

    public void SendDisappearing(string chatId, string text, ulong expiresAtSecs) =>
        _ffi.Dispatch(new AppAction.SendDisappearingMessage(chatId, text, expiresAtSecs));

    public void SetChatMessageTtl(string chatId, ulong? ttlSeconds) =>
        _ffi.Dispatch(new AppAction.SetChatMessageTtl(chatId, ttlSeconds));

    public void ToggleReaction(string chatId, string messageId, string emoji) =>
        _ffi.Dispatch(new AppAction.ToggleReaction(chatId, messageId, emoji));

    public void DeleteLocalMessage(string chatId, string messageId) =>
        _ffi.Dispatch(new AppAction.DeleteLocalMessage(chatId, messageId));

    public void MarkMessagesSeen(string chatId, string[] messageIds) =>
        _ffi.Dispatch(new AppAction.MarkMessagesSeen(chatId, messageIds));

    public void SendTyping(string chatId) =>
        _ffi.Dispatch(new AppAction.SendTyping(chatId));

    public void StopTyping(string chatId) =>
        _ffi.Dispatch(new AppAction.StopTyping(chatId));

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
            _ffi.Dispatch(new AppAction.SendAttachments(c, staged, caption?.Trim() ?? string.Empty));
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
        _ffi.Dispatch(new AppAction.CreatePublicInvite());

    public void AcceptInvite(string inviteInput)
    {
        var t = inviteInput.Trim();
        if (string.IsNullOrEmpty(t)) return;
        _ffi.Dispatch(new AppAction.AcceptInvite(t));
    }

    public void CreateGroup(string name, IList<string> memberInputs)
    {
        var n = name.Trim();
        if (string.IsNullOrEmpty(n) || memberInputs == null || memberInputs.Count == 0) return;
        _ffi.Dispatch(new AppAction.CreateGroup(n, memberInputs.Select(s => s.Trim()).Where(s => s.Length > 0).ToArray()));
    }

    public void UpdateGroupName(string groupId, string name) =>
        _ffi.Dispatch(new AppAction.UpdateGroupName(groupId, name.Trim()));

    public void UpdateGroupPicture(string groupId, string sourceFilePath)
    {
        try
        {
            var staged = _cache.StageOutgoing(sourceFilePath);
            _ffi.Dispatch(new AppAction.UpdateGroupPicture(groupId, staged.Path, staged.Filename));
        }
        catch
        {
            ShowToast("Image could not be opened");
        }
    }

    public void AddGroupMembers(string groupId, IList<string> memberInputs) =>
        _ffi.Dispatch(new AppAction.AddGroupMembers(
            groupId,
            memberInputs.Select(s => s.Trim()).Where(s => s.Length > 0).ToArray()
        ));

    public void SetGroupAdmin(string groupId, string ownerPubkeyHex, bool isAdmin) =>
        _ffi.Dispatch(new AppAction.SetGroupAdmin(groupId, ownerPubkeyHex, isAdmin));

    public void RemoveGroupMember(string groupId, string ownerPubkeyHex) =>
        _ffi.Dispatch(new AppAction.RemoveGroupMember(groupId, ownerPubkeyHex));

    public void DeleteChat(string chatId) =>
        _ffi.Dispatch(new AppAction.DeleteChat(chatId));

    // ─────────────────────────────── settings ─────────────────────────────────

    public void SetTypingIndicatorsEnabled(bool enabled) =>
        _ffi.Dispatch(new AppAction.SetTypingIndicatorsEnabled(enabled));

    public void SetReadReceiptsEnabled(bool enabled) =>
        _ffi.Dispatch(new AppAction.SetReadReceiptsEnabled(enabled));

    public void SetDesktopNotificationsEnabled(bool enabled) =>
        _ffi.Dispatch(new AppAction.SetDesktopNotificationsEnabled(enabled));

    public void SetStartupAtLoginEnabled(bool enabled)
    {
        try
        {
            PlatformStartupAtLogin.SetEnabled(enabled);
            _ffi.Dispatch(new AppAction.SetStartupAtLoginEnabled(enabled));
        }
        catch
        {
            ShowToast("Startup setting unavailable");
        }
    }

    public void AddNostrRelay(string url) =>
        _ffi.Dispatch(new AppAction.AddNostrRelay(url.Trim()));

    public void UpdateNostrRelay(string oldUrl, string newUrl) =>
        _ffi.Dispatch(new AppAction.UpdateNostrRelay(oldUrl.Trim(), newUrl.Trim()));

    public void RemoveNostrRelay(string url) =>
        _ffi.Dispatch(new AppAction.RemoveNostrRelay(url.Trim()));

    public void ResetNostrRelays() =>
        _ffi.Dispatch(new AppAction.ResetNostrRelays());

    public void SetImageProxyEnabled(bool enabled) =>
        _ffi.Dispatch(new AppAction.SetImageProxyEnabled(enabled));

    public void SetImageProxyUrl(string url) =>
        _ffi.Dispatch(new AppAction.SetImageProxyUrl(url.Trim()));

    public void SetImageProxyKeyHex(string keyHex) =>
        _ffi.Dispatch(new AppAction.SetImageProxyKeyHex(keyHex.Trim()));

    public void SetImageProxySaltHex(string saltHex) =>
        _ffi.Dispatch(new AppAction.SetImageProxySaltHex(saltHex.Trim()));

    public void ResetImageProxySettings() =>
        _ffi.Dispatch(new AppAction.ResetImageProxySettings());

    // ────────────────────── support / build metadata ─────────────────────────

    public string SupportBundleJson() => _ffi.ExportSupportBundleJson();
    public string BuildSummary() => Native.BuildSummary();
    public string RelaySetIdText() => Native.RelaySetId();
    public bool TrustedTestBuildEnabled() => Native.IsTrustedTestBuild();
    public string? ExportOwnerNsec() => _secretStore.Load()?.OwnerNsec;
    public string? ExportDeviceNsec() => _secretStore.Load()?.DeviceNsec;

    public void CopyToClipboard(string value)
    {
        PlatformClipboard.SetString(value);
        ShowToast("Copied");
    }

    // ───────────────────────────── plumbing ───────────────────────────────────

    private void TryRestorePersistedSession()
    {
        var bundle = _secretStore.Load();
        if (bundle == null)
        {
            BootstrapInFlight = false;
            Notify(nameof(BootstrapInFlight));
            return;
        }
        _ffi.Dispatch(new AppAction.RestoreAccountBundle(
            bundle.OwnerNsec,
            bundle.OwnerPubkeyHex,
            bundle.DeviceNsec
        ));
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
                break;

            case AppUpdate.FullState f:
                if (f.v1.rev <= _lastRevApplied) return;
                var prev = _state;
                _state = f.v1;
                _lastRevApplied = f.v1.rev;
                BootstrapInFlight = false;

                PostDesktopNotifications(prev, f.v1);

                NotifyAll();

                if (!string.IsNullOrEmpty(f.v1.toast))
                {
                    ShowToast(f.v1.toast!);
                }
                break;
        }
    }

    private void PostDesktopNotifications(AppState old, AppState next)
    {
        if (old.account == null) return;
        if (!next.preferences.desktopNotificationsEnabled) return;

        // Suppress while the user is looking at our window. iOS/macOS get this
        // for free from UNUserNotificationCenter; on Windows we have to ask
        // WPF whether our main window is currently the foreground window.
        var mainWindow = Application.Current?.MainWindow;
        if (mainWindow != null && mainWindow.IsActive) return;

        var oldUnread = (old.chatList ?? Array.Empty<ChatThreadSnapshot>())
            .ToDictionary(c => c.chatId, c => c.unreadCount);
        var currentChatId = next.currentChat?.chatId;

        foreach (var chat in next.chatList ?? Array.Empty<ChatThreadSnapshot>())
        {
            if (chat.lastMessageIsOutgoing == true) continue;
            if (chat.chatId == currentChatId) continue;
            oldUnread.TryGetValue(chat.chatId, out var prevUnread);
            if (chat.unreadCount <= prevUnread) continue;

            var preview = chat.lastMessagePreview?.Trim();
            var body = string.IsNullOrEmpty(preview) ? "New message" : preview!;
            try { _notifier.Post(chat.displayName, body); } catch { }
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
    }

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

    private sealed class Reconciler : AppReconciler
    {
        private readonly AppManager _owner;
        public Reconciler(AppManager owner) { _owner = owner; }

        public void Reconcile(AppUpdate update)
        {
            _owner._ui.BeginInvoke(new Action(() => _owner.Apply(update)));
        }
    }
}
