using System;
using System.Linq;
using System.Windows.Threading;
using System.Windows;
using System.Windows.Interop;
using System.Threading.Tasks;
using IrisChat.Bindings;

namespace IrisChat;

// Owns codecs/devices only. Signalling and compressed media use the existing core actions.
public sealed class CallController : IDisposable
{
    private readonly AppManager _manager;
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromMilliseconds(15) };
    private DesktopCallMedia? _media;
    private DesktopCallTone? _tone;
    private string? _ending;
    private ScreenCapture? _screen;
    internal Func<IntPtr, Task<Windows.Graphics.Capture.GraphicsCaptureItem?>> ScreenPicker { get; set; } = ScreenCapture.PickAsync;
    private long _screenRequest;
    private bool _cameraBeforeSharing;
    private bool? _videoOverride;
    public bool SharingScreen => _screen != null;
    public bool ChoosingScreen { get; private set; }
    public async Task ToggleScreenShareAsync(Window window)
    {
        if (_screen != null) { StopScreenShare(true); return; }
        if (ChoosingScreen || Call is not { phase: "connected", videoCapable: true } call || _media == null) return;
        var request = ++_screenRequest;
        ChoosingScreen = true; Changed?.Invoke();
        try
        {
            var item = await ScreenPicker(new WindowInteropHelper(window).Handle);
            if (item == null || request != _screenRequest || Call?.callId != call.callId || Call.phase != "connected" || _media == null) return;
            var media = _media;
            var capture = new ScreenCapture(item, (width, height, bytes) => media.SubmitVideoFrame(width, height, bytes),
                () => _timer.Dispatcher.BeginInvoke(() => { if (request == _screenRequest) StopScreenShare(true); }));
            _cameraBeforeSharing = Call.video;
            _screen = capture;
            media.SetExternalVideo(true);
            _videoOverride = true;
            media.Configure(Call.muted, true, Call.targetBitrateBps, Call.keyFrameGeneration);
            try { capture.Start(); }
            catch { StopScreenShare(true); throw; }
            _manager.DispatchCall(new AppAction.SetCallVideoEnabled(true));
        }
        catch (Exception) { if (Call?.callId == call.callId && Call.phase == "connected" && _ending != call.callId) _manager.ShowToast("Couldn’t share the screen."); }
        finally { if (request == _screenRequest) { ChoosingScreen = false; Changed?.Invoke(); } }
    }
    private void StopScreenShare(bool restoreCamera)
    {
        ++_screenRequest;
        ChoosingScreen = false;
        var screen = _screen; _screen = null;
        screen?.Dispose();
        if (screen != null && restoreCamera && Call is { phase: "connected" } call && _media != null)
        {
            _videoOverride = _cameraBeforeSharing;
            _media.Configure(call.muted, _cameraBeforeSharing, call.targetBitrateBps, call.keyFrameGeneration);
            _manager.DispatchCall(new AppAction.SetCallVideoEnabled(_cameraBeforeSharing));
        }
        // During hangup the media engine is stopped immediately afterward;
        // do not briefly reopen the camera while dismantling a screen share.
        if (restoreCamera) _media?.SetExternalVideo(false);
        if (screen != null) Changed?.Invoke();
    }
    private DateTime _lastRing;
    private DateTime _lastAudioDevices;
    public DesktopAudioDevices? AudioDevices { get; private set; }
    public CallSnapshot? Call { get; private set; }
    public event Action? Changed;
    public event Action<DesktopCallEvent.Video>? Video;
    public void SelectAudioDevices(string microphone, string speaker) => _media?.SelectAudioDevices(microphone, speaker);

    public CallController(AppManager manager)
    {
        _manager = manager;
        _timer.Tick += (_, _) => Poll();
        _timer.Start();
    }
    public void Update(CallSnapshot? call)
    {
        if (Call?.callId != call?.callId) { StopTone(); StopMedia(); _ending = null; }
        Call = call;
        AudioMessagePlayback.SetCallActive(call != null && call.phase != "ended" && _ending != call.callId);
        if (call?.outgoing == true && _ending != call.callId && call.phase is "outgoing" or "ringing")
        {
            _tone ??= new DesktopCallTone(call.phase == "ringing");
            _tone.SetRinging(call.phase == "ringing");
        }
        else StopTone();
        if (call?.phase == "connected" && _ending != call.callId)
        {
            _media ??= new DesktopCallMedia();
            if (_videoOverride == call.video) _videoOverride = null;
            _media.Configure(call.muted, _videoOverride ?? call.video, call.targetBitrateBps, call.keyFrameGeneration);
        }
        else StopMedia();
        Changed?.Invoke();
    }
    public void Receive(AppUpdate.CallMedia frame)
    {
        if (Call?.phase == "connected" && Call.callId == frame.callId && _ending != frame.callId)
            _media?.Receive(frame.kind, frame.sequence, frame.timestampUs, frame.keyFrame, frame.data);
    }
    public void End()
    {
        if (Call == null) return;
        _ending = Call.callId;
        AudioMessagePlayback.SetCallActive(false);
        StopTone();
        StopMedia();
        _manager.DispatchCall(new AppAction.EndCall(Call.callId));
        Changed?.Invoke();
    }
    public bool Visible => Call != null && _ending != Call.callId;
    private void Poll()
    {
        if (Call?.phase == "incoming" && Visible && DateTime.UtcNow - _lastRing > TimeSpan.FromSeconds(3))
        {
            _lastRing = DateTime.UtcNow;
            System.Media.SystemSounds.Exclamation.Play();
        }
        if (_media == null || Call == null) return;
        if (DateTime.UtcNow - _lastAudioDevices > TimeSpan.FromSeconds(1))
        {
            _lastAudioDevices = DateTime.UtcNow;
            var devices = _media.AudioDevices();
            var old = AudioDevices;
            if (old == null || old.microphone != devices.microphone || old.speaker != devices.speaker || old.error != devices.error ||
                !old.microphones.SequenceEqual(devices.microphones) || !old.speakers.SequenceEqual(devices.speakers))
            {
                AudioDevices = devices;
                if (devices.error != null && devices.error != old?.error) _manager.ShowToast(devices.error);
                Changed?.Invoke();
            }
        }
        var id = Call.callId;
        foreach (var item in _media.Poll())
        {
            switch (item)
            {
                case DesktopCallEvent.Encoded e:
                    _manager.DispatchCall(new AppAction.SendCallMedia(id, e.kind, e.timestampUs, e.keyFrame, e.data)); break;
                case DesktopCallEvent.Video v: Video?.Invoke(v); break;
                case DesktopCallEvent.Ready:
                    _manager.DispatchCall(new AppAction.SetCallMediaConnected(id, true)); break;
                case DesktopCallEvent.RequestKeyFrame:
                    _manager.DispatchCall(new AppAction.RequestCallKeyFrame(id)); break;
                case DesktopCallEvent.Error e:
                    End(); _manager.ShowToast(e.message); return;
            }
        }
    }
    private void StopMedia() { StopScreenShare(false); _videoOverride = null; _media?.Stop(); _media?.Dispose(); _media = null; AudioDevices = null; }
    private void StopTone() { _tone?.Stop(); _tone?.Dispose(); _tone = null; }
    public void Dispose() { AudioMessagePlayback.SetCallActive(false); _timer.Stop(); StopTone(); StopMedia(); }
}
