using System;
using System.Reflection;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Bindings;
using Windows.Graphics.Capture;

internal static class ScreenShareTests
{
    private static readonly BindingFlags Hidden = BindingFlags.Instance | BindingFlags.NonPublic;
    internal static void Run(AppManager manager, Window window, CallSnapshot fixture)
    {
        var calls = manager.Calls;
        var timer = (DispatcherTimer)typeof(CallController).GetField("_timer", Hidden)!.GetValue(calls)!;
        var picker = typeof(CallController).GetProperty("ScreenPicker", Hidden)!;
        var originalPicker = picker.GetValue(calls);
        timer.Stop();
        try
        {
            var snapshot = fixture with { callId = "screen-picker-test", phase = "connected", video = false, muted = true };
            // Device errors are intentionally not polled in this picker-only fixture.
            // Production H.264/source switching has a separate hardware-free Rust test.
            calls.Update(snapshot);
            var media = typeof(CallController).GetField("_media", Hidden)!.GetValue(calls);
            picker.SetValue(calls, (Func<IntPtr, Task<GraphicsCaptureItem?>>)(_ => Task.FromResult<GraphicsCaptureItem?>(null)));
            calls.ToggleScreenShareAsync(window).GetAwaiter().GetResult();
            Check(calls.Call == snapshot && !calls.ChoosingScreen && !calls.SharingScreen, "Cancel preserves the camera, mute and call");
            Check(ReferenceEquals(media, typeof(CallController).GetField("_media", Hidden)!.GetValue(calls)), "Cancel preserves the running media engine");
            picker.SetValue(calls, (Func<IntPtr, Task<GraphicsCaptureItem?>>)(_ => Task.FromException<GraphicsCaptureItem?>(new InvalidOperationException("Capture unavailable"))));
            calls.ToggleScreenShareAsync(window).GetAwaiter().GetResult();
            Check(calls.Call == snapshot && !calls.ChoosingScreen && !calls.SharingScreen, "Unavailable capture preserves the call");
            var pending = new TaskCompletionSource<GraphicsCaptureItem?>();
            picker.SetValue(calls, (Func<IntPtr, Task<GraphicsCaptureItem?>>)(_ => pending.Task));
            var choosing = calls.ToggleScreenShareAsync(window);
            Check(calls.ChoosingScreen, "Pending picker disables duplicate selection");
            calls.End();
            pending.SetResult(null);
            choosing.GetAwaiter().GetResult();
            Check(!calls.Visible && !calls.ChoosingScreen && !calls.SharingScreen, "Hangup invalidates pending picker");
            Check(typeof(CallController).GetField("_media", Hidden)!.GetValue(calls) == null, "Late picker cannot recreate media");
            calls.Update(null);
            Console.WriteLine("PASS: screen picker cancel, unavailable capture, preserved audio/camera and late-result hangup");
        }
        finally { picker.SetValue(calls, originalPicker); timer.Start(); }
    }
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); }
}
