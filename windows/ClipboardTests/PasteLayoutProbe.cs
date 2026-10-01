using System;
using System.Diagnostics;
using System.Windows.Media;
using System.Windows.Threading;

// Input-priority heartbeats expose a long layout turn even when Paste itself
// returns quickly. Rendering is the WPF frame callback, not display scanout.
internal sealed class PasteLayoutProbe : IDisposable
{
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private readonly DispatcherTimer heartbeat = new(DispatcherPriority.Input)
    {
        Interval = TimeSpan.FromMilliseconds(1),
    };
    private double lastHeartbeatMs;
    private readonly Action? firstRender;
    private DispatcherFrame? renderWait;
    private bool finished;

    internal double ElapsedMs => elapsed.Elapsed.TotalMilliseconds;
    internal double? FirstRenderMs { get; private set; }
    internal double LongestDispatcherGapMs { get; private set; }

    internal PasteLayoutProbe(Action? firstRender = null)
    {
        this.firstRender = firstRender;
        heartbeat.Tick += OnHeartbeat;
        CompositionTarget.Rendering += OnRendering;
        heartbeat.Start();
    }

    private void OnHeartbeat(object? sender, EventArgs args) => RecordGap();

    private void RecordGap()
    {
        var now = ElapsedMs;
        LongestDispatcherGapMs = Math.Max(LongestDispatcherGapMs, now - lastHeartbeatMs);
        lastHeartbeatMs = now;
    }

    private void OnRendering(object? sender, EventArgs args)
    {
        if (FirstRenderMs == null)
        {
            FirstRenderMs = ElapsedMs;
            firstRender?.Invoke();
        }
        if (renderWait != null) renderWait.Continue = false;
    }

    internal double CheckpointGap()
    {
        RecordGap();
        return LongestDispatcherGapMs;
    }

    internal void WaitForFirstRender()
    {
        if (FirstRenderMs != null) return;
        renderWait = new DispatcherFrame();
        var timeout = new DispatcherTimer(DispatcherPriority.Send)
        {
            Interval = TimeSpan.FromSeconds(5),
        };
        timeout.Tick += (_, _) => { if (renderWait != null) renderWait.Continue = false; };
        timeout.Start();
        try { Dispatcher.PushFrame(renderWait); }
        finally { timeout.Stop(); renderWait = null; }
        if (FirstRenderMs == null) throw new InvalidOperationException("Pasted composer did not reach a WPF rendering callback");
    }

    internal void Finish()
    {
        if (finished) return;
        finished = true;
        RecordGap();
        heartbeat.Stop();
        heartbeat.Tick -= OnHeartbeat;
        CompositionTarget.Rendering -= OnRendering;
        elapsed.Stop();
    }

    public void Dispose() => Finish();
}
