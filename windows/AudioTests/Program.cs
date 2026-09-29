using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Xml.Linq;
using System.Windows.Markup;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Chrome;

internal static class Program
{
    [STAThread]
    private static int Main(string[] args)
    {
        var output = Path.GetFullPath(args.Length > 0 ? args[0] : "work/audio-ui");
        Directory.CreateDirectory(output);
        using var log = new StreamWriter(Path.Combine(output, "audio-test.log")) { AutoFlush = true };
        Console.SetOut(log); Console.SetError(log);
        Trace.Listeners.Add(new TextWriterTraceListener(log)); Trace.AutoFlush = true;
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(Dispatcher.CurrentDispatcher));
        XNamespace xaml = "http://schemas.microsoft.com/winfx/2006/xaml/presentation";
        var resources = XDocument.Load(Path.Combine(AppContext.BaseDirectory, "App.xaml")).Descendants(xaml + "ResourceDictionary").Single();
        resources.SetAttributeValue(XNamespace.Xmlns + "x", "http://schemas.microsoft.com/winfx/2006/xaml");
        app.Resources = (ResourceDictionary)XamlReader.Parse(resources.ToString());
        var column = new StackPanel { Margin = new Thickness(24) };
        var window = new Window { Title = "Voice messages", Width = 580, Height = 280, Content = column, Background = new SolidColorBrush(Color.FromRgb(18,18,18)) };
        var bytes = File.ReadAllBytes(Path.Combine(AppContext.BaseDirectory, "voice-message.m4a"));
        var downloads = 0;
        Task<byte[]?> Download() { downloads++; return Task.FromResult<byte[]?>(bytes); }
        var incoming = new AudioMessageControl("incoming", "Voice message.m4a", Download);
        var outgoing = new AudioMessageControl("outgoing", "Voice message.m4a", Download);
        Border Bubble(AudioMessageControl control, bool mine) => new() {
            Child = control, Padding = new Thickness(12), CornerRadius = new CornerRadius(16), Margin = new Thickness(0,6,0,6),
            HorizontalAlignment = mine ? HorizontalAlignment.Right : HorizontalAlignment.Left,
            Background = new SolidColorBrush(mine ? Color.FromRgb(112,40,205) : Color.FromRgb(45,45,45)),
        };
        var firstBubble = Bubble(incoming, false);
        column.Children.Add(firstBubble); column.Children.Add(Bubble(outgoing, true));
        try
        {
            window.Show(); PumpUntil(() => incoming.IsLoaded && outgoing.IsLoaded);
            Check(downloads == 0, "No download before Play");
            Check(AudioMessagePlayback.IsAudio(false, "Voice.M4A"), "Old messages are recognized by extension");
            Check(!AudioMessagePlayback.IsAudio(false, "file.pdf"), "Other attachments are unchanged");
            var peaks = AudioWaveform.Decode(Path.Combine(AppContext.BaseDirectory, "voice-message.m4a"));
            Check(peaks.Length == 47, "Native M4A decoder returns 47 waveform peaks");
            Check(peaks.Skip(18).Take(4).All(p => p < 0.02), "Real silence stays flat");
            Check(peaks.Take(14).Any(p => p > 0.5), "Real audio has peaks");
            TestCancellationAndRetry();
            TestImageClipboard(output);
            AttachmentDropTests.Run();
            Save(column, Path.Combine(output, "windows-inline-voice-messages.png"));
            Click(incoming, "chatAudioPlayButton");
            if (args.Length > 1 && args[1] == "--expect-no-audio-device")
            {
                PumpUntil(() => Name(incoming, "chatAudioPlayButton") == "Retry audio");
                Check(!Find<Slider>(incoming, "chatAudioProgress").IsEnabled, "Failed audio cannot seek");
                Click(incoming, "chatAudioSpeedButton");
                Check(Find<Button>(incoming,"chatAudioSpeedButton").Content?.ToString() == "1.5×", "Speed button responds");
                Click(incoming, "chatAudioPlayButton");
                PumpUntil(() => Name(incoming, "chatAudioPlayButton") == "Retry audio");
                Check(downloads == 2, "Unavailable audio retries in the app");
                Check(Find<WaveformSlider>(incoming, "chatAudioProgress").Peaks.Count == 47, "Decoded waveform reaches message UI");
                Save(column, Path.Combine(output, "windows-audio-unavailable.png"));
                Console.WriteLine("PASS: Media Foundation M4A waveform decode/silence, WPF controls, cancellation, retry, call interruption and in-app device error");
                Console.WriteLine("SKIP: real audio playback requires a Windows audio output device");
                return 77;
            }
            PumpUntil(() => Name(incoming, "chatAudioPlayButton") == "Pause audio");
            var slider = Find<Slider>(incoming, "chatAudioProgress");
            PumpUntil(() => slider.Value > 0.1);
            Check(Math.Abs(slider.Maximum - 6) < 0.2, "M4A duration");
            Click(incoming, "chatAudioPlayButton");
            Check(Name(incoming, "chatAudioPlayButton") == "Play audio", "Pause");
            slider.Value = 3;
            Check(Math.Abs(slider.Value - 3) < 0.1, "Seek while paused");
            foreach (var speed in new[] { "1.5×", "2×", "0.5×", "1×" })
            {
                Click(incoming, "chatAudioSpeedButton");
                Check(Find<Button>(incoming,"chatAudioSpeedButton").Content?.ToString() == speed, "Speed cycle");
                Check(Name(incoming, "chatAudioPlayButton") == "Play audio", "Speed does not start paused audio");
            }
            Click(incoming, "chatAudioPlayButton");
            PumpUntil(() => Name(incoming, "chatAudioPlayButton") == "Pause audio");
            Check(downloads == 1, "Resume uses loaded audio");
            // A receipt refresh recreates the message row within one dispatch.
            firstBubble.Child = null;
            incoming = new AudioMessageControl("incoming", "Voice message.m4a", Download);
            firstBubble.Child = incoming;
            PumpUntil(() => incoming.IsLoaded);
            Check(Name(incoming, "chatAudioPlayButton") == "Pause audio", "Row replacement retains playback");
            Click(outgoing,"chatAudioPlayButton");
            PumpUntil(() => Name(outgoing,"chatAudioPlayButton") == "Pause audio");
            Check(Name(incoming,"chatAudioPlayButton") == "Play audio", "Only one message plays");
            AudioMessagePlayback.SetCallActive(true);
            Check(Name(outgoing,"chatAudioPlayButton") == "Play audio", "Incoming call pauses audio");
            AudioMessagePlayback.SetCallActive(false);
            Click(outgoing,"chatAudioPlayButton");
            Find<Slider>(outgoing,"chatAudioProgress").Value = 5.8;
            PumpUntil(() => Name(outgoing,"chatAudioPlayButton") == "Play audio");
            Click(outgoing,"chatAudioPlayButton");
            PumpUntil(() => Name(outgoing,"chatAudioPlayButton") == "Pause audio" && Find<Slider>(outgoing,"chatAudioProgress").Value < 1);
            Click(outgoing,"chatAudioPlayButton");
            Click(incoming,"chatAudioSpeedButton");
            Save(column,Path.Combine(output,"windows-inline-voice-messages.png"));

            window.Close(); Pump();
            Console.WriteLine("PASS: WPF M4A decode, progress, pause, seek, speed, replay, single playback, calls, row replacement, cancellation and retry");
            return 0;
        }
        catch(Exception e) { Console.Error.WriteLine(e); return 1; }
        finally { window.Close(); app.Shutdown(); }
    }

    private static void TestImageClipboard(string output)
    {
        var pixels = new byte[] { 255, 0, 0, 255, 0, 255, 0, 255 };
        var original = BitmapSource.Create(2, 1, 96, 96, PixelFormats.Bgra32, null, pixels, 8);
        var png = new PngBitmapEncoder(); png.Frames.Add(BitmapFrame.Create(original));
        using var encoded = new MemoryStream(); png.Save(encoded);
        var decoded = ImageViewerWindow.Decode(encoded.ToArray())!;
        var viewer = new ImageViewerWindow(decoded, "Image clipboard test");
        try
        {
            viewer.Show(); PumpUntil(() => viewer.IsLoaded);
            var image = (Image)viewer.Content;
            ((MenuItem)image.ContextMenu.Items[0]).RaiseEvent(new RoutedEventArgs(MenuItem.ClickEvent));
            var copied = Clipboard.GetImage();
            Check(copied != null && copied.PixelWidth == 2 && copied.PixelHeight == 1, "Image clipboard contains full pixels, not a link");
            var actual = new byte[8]; copied!.CopyPixels(actual, 8, 0);
            Check(actual.SequenceEqual(pixels), "Copied image pixels are unchanged");
            Save(image, Path.Combine(output, "windows-image-viewer.png"));
            Console.WriteLine("PASS: opened image viewer copies actual image pixels");
        }
        finally { viewer.Close(); }
    }

    private static void TestCancellationAndRetry()
    {
        var pending = new TaskCompletionSource<byte[]?>();
        var playback = AudioMessagePlayback.Attach("pending", "voice.m4a", () => pending.Task);
        var load = playback.PlayAsync();
        playback.Pause();
        pending.SetResult(new byte[] { 1, 2, 3 });
        PumpUntil(() => load.IsCompleted);
        Check(!playback.IsPlaying && !playback.IsLoading && playback.Error == null, "Canceled download cannot autoplay");
        playback.Dispose();
        var attempts = 0;
        playback = AudioMessagePlayback.Attach("retry", "voice.m4a", () => { attempts++; return Task.FromResult<byte[]?>(null); });
        _ = playback.PlayAsync();
        PumpUntil(() => playback.Error != null);
        _ = playback.PlayAsync();
        PumpUntil(() => playback.Error != null && attempts == 2);
        playback.Dispose();
        pending = new TaskCompletionSource<byte[]?>();
        playback = AudioMessagePlayback.Attach("call", "voice.m4a", () => pending.Task);
        load = playback.PlayAsync();
        Check(playback.IsLoading, "Call test starts download");
        AudioMessagePlayback.SetCallActive(true);
        Check(!playback.IsLoading && !playback.IsPlaying, "Call cancels pending audio");
        _ = playback.PlayAsync();
        Check(playback.Error == "Finish your call to play audio.", "Calls prevent new playback");
        pending.SetResult(null);
        PumpUntil(() => load.IsCompleted);
        playback.Dispose();
        AudioMessagePlayback.SetCallActive(false);
    }
    private static void Check(bool condition,string message) { if(!condition) throw new Exception(message); }
    private static T Find<T>(DependencyObject root,string id) where T:FrameworkElement
    {
        if(root is T match && AutomationProperties.GetAutomationId(match)==id) return match;
        for(var i=0;i<VisualTreeHelper.GetChildrenCount(root);i++) {
            var found=FindOrNull<T>(VisualTreeHelper.GetChild(root,i),id); if(found!=null) return found;
        }
        throw new Exception("Missing control: "+id);
    }
    private static T? FindOrNull<T>(DependencyObject root,string id) where T:FrameworkElement {
        if(root is T match && AutomationProperties.GetAutomationId(match)==id) return match;
        for(var i=0;i<VisualTreeHelper.GetChildrenCount(root);i++) {var found=FindOrNull<T>(VisualTreeHelper.GetChild(root,i),id);if(found!=null)return found;} return null;
    }
    private static string Name(DependencyObject root,string id)=>AutomationProperties.GetName(Find<Button>(root,id));
    private static void Click(DependencyObject root,string id)=>Find<Button>(root,id).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
    private static void Pump() { var f=new DispatcherFrame(); Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ContextIdle,new Action(()=>f.Continue=false));Dispatcher.PushFrame(f); }
    private static void PumpUntil(Func<bool> ready) {var timer=Stopwatch.StartNew(); do {Pump(); if(ready())return; Thread.Sleep(2);} while(timer.Elapsed<TimeSpan.FromSeconds(12));throw new Exception("Audio state timed out");}
    private static void Save(FrameworkElement view,string path) {
        view.UpdateLayout();var bitmap=new RenderTargetBitmap((int)view.ActualWidth,(int)view.ActualHeight,96,96,PixelFormats.Pbgra32);var visual = new DrawingVisual();
        using (var drawing = visual.RenderOpen()) drawing.DrawRectangle(new VisualBrush(view), null, new Rect(0, 0, view.ActualWidth, view.ActualHeight));
        bitmap.Render(visual);
        var png=new PngBitmapEncoder();png.Frames.Add(BitmapFrame.Create(bitmap));using var stream=File.Create(path);png.Save(stream);
    }
}
