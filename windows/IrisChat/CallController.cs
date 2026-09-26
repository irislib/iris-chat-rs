using System;
using System.Windows.Threading;
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
    private DateTime _lastRing;
    public CallSnapshot? Call { get; private set; }
    public event Action? Changed;
    public event Action<DesktopCallEvent.Video>? Video;

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
        if (call?.outgoing == true && _ending != call.callId && call.phase is "outgoing" or "ringing")
        {
            _tone ??= new DesktopCallTone(call.phase == "ringing");
            _tone.SetRinging(call.phase == "ringing");
        }
        else StopTone();
        if (call?.phase == "connected" && _ending != call.callId)
        {
            _media ??= new DesktopCallMedia();
            _media.Configure(call.muted, call.video, call.targetBitrateBps, call.keyFrameGeneration);
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
    private void StopMedia() { _media?.Stop(); _media?.Dispose(); _media = null; }
    private void StopTone() { _tone?.Stop(); _tone?.Dispose(); _tone = null; }
    public void Dispose() { _timer.Stop(); StopTone(); StopMedia(); }
}
