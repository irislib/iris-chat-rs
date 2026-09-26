using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using IrisChat.Bindings;

namespace IrisChat.Views;
public partial class CallView : UserControl
{
    private string? _id;
    private CallController? _calls;
    public CallView()
    {
        InitializeComponent();
        Loaded += (_, _) => { _calls = App.CurrentManager.Calls; _calls.Changed += Refresh; _calls.Video += ShowVideo; Refresh(); };
        Unloaded += (_, _) => { if (_calls != null) { _calls.Changed -= Refresh; _calls.Video -= ShowVideo; } _calls = null; };
    }
    private static Visibility Show(bool value) => value ? Visibility.Visible : Visibility.Collapsed;
    private void Refresh()
    {
        Visibility = Show(_calls?.Visible == true);
        var c = _calls?.Call;
        if (c == null) { RemoteVideo.Source = LocalVideo.Source = null; return; }
        if (_id != c.callId) { _id = c.callId; RemoteVideo.Source = LocalVideo.Source = null; }
        var incoming = c.phase == "incoming";
        var connected = c.phase == "connected";
        Peer.Text = c.peerName;
        Initial.Text = string.IsNullOrWhiteSpace(c.peerName) ? "?" : c.peerName.Trim()[..1].ToUpperInvariant();
        Avatar.Visibility = Show(!connected || !c.remoteVideo);
        Status.Text = c.phase == "ended" ? c.endReason ?? "Call ended" : incoming ? (c.videoCapable ? "Incoming video call" : "Incoming voice call") : c.phase == "ringing" ? "Ringing…" : !connected ? "Calling…" : !c.mediaConnected ? "Connecting…" : c.remoteMuted ? "Microphone muted" : "Connected";
        Answer.Visibility = Show(incoming); Voice.Visibility = Show(incoming && c.videoCapable);
        Mute.Visibility = Show(connected); Mute.Content = c.muted ? "Unmute" : "Mute";
        Camera.Visibility = Show(connected && c.videoCapable); Camera.Content = c.video ? "Camera off" : "Camera on";
        End.Content = c.phase == "ended" ? "Done" : incoming ? "Decline" : "End call";
        LocalVideo.Visibility = Show(connected && c.video); RemoteVideo.Visibility = Show(connected && c.remoteVideo);
    }
    private void ShowVideo(DesktopCallEvent.Video frame)
    {
        // Rust emits RGBA, WPF consumes BGRA. Own the FFI array before rendering.
        var bytes = frame.rgba;
        for (int i = 0; i + 3 < bytes.Length; i += 4) (bytes[i], bytes[i+2]) = (bytes[i+2], bytes[i]);
        var image = frame.local ? LocalVideo : RemoteVideo;
        if (image.Source is not WriteableBitmap bitmap || bitmap.PixelWidth != frame.width || bitmap.PixelHeight != frame.height)
        {
            bitmap = new WriteableBitmap((int)frame.width, (int)frame.height, 96, 96, PixelFormats.Bgra32, null);
            image.Source = bitmap;
        }
        bitmap.WritePixels(new Int32Rect(0, 0, (int)frame.width, (int)frame.height), bytes, (int)frame.width * 4, 0);
    }
    private void OnAnswer(object sender, RoutedEventArgs e) { if (_calls?.Call is {} c) App.CurrentManager.DispatchCall(new AppAction.AnswerCall(c.callId)); }
    private void OnVoice(object sender, RoutedEventArgs e) { if (_calls?.Call is {} c) App.CurrentManager.DispatchCall(new AppAction.AnswerCallWithVoice(c.callId)); }
    private void OnEnd(object sender, RoutedEventArgs e) => _calls?.End();
    private void OnMute(object sender, RoutedEventArgs e) { if (_calls?.Call is {} c) App.CurrentManager.DispatchCall(new AppAction.SetCallMuted(!c.muted)); }
    private void OnCamera(object sender, RoutedEventArgs e) { if (_calls?.Call is {} c) App.CurrentManager.DispatchCall(new AppAction.SetCallVideoEnabled(!c.video)); }
}
