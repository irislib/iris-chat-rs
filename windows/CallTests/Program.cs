using System;
using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Views;

internal static class Program
{
    [STAThread]
    private static int Main(string[] args)
    {
        var output = Path.GetFullPath(args.Length > 0 ? args[0] : "work/call-ui");
        Directory.CreateDirectory(output);
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_RUN_ID", Guid.NewGuid().ToString());
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_DATA_DIR", Path.Combine(output, "data"));
        var app = new App();
        app.InitializeComponent();
        var manager = new AppManager(Path.Combine(output, "data"), new SilentNotifications(), Path.Combine(output,"secret.json"));
        typeof(App).GetProperty(nameof(App.Manager))!.SetValue(app, manager);
        var view = new CallView();
        var window = new Window { Title = "Call UI test", Width = 620, Height = 520, Content = view };
        try
        {
            window.Show(); Pump();
            var call = new CallSnapshot(false,150000,0,false,1200000,"test-call","test-peer","Alex","incoming",true,true,false,true,false,0,null,null);
            manager.Calls.Update(call); Pump();
            Check(view.Visibility == Visibility.Visible, "Incoming call must be visible");
            Check(((Button)view.FindName("Answer")).Visibility == Visibility.Visible, "Answer button");
            Check(((Button)view.FindName("Voice")).Visibility == Visibility.Visible, "Voice-only answer");
            Check(((Button)view.FindName("Camera")).Visibility == Visibility.Collapsed, "Camera must remain off before answer");
            Check(typeof(CallController).GetField("_media",BindingFlags.NonPublic|BindingFlags.Instance)!.GetValue(manager.Calls)==null,"No devices before answer");
            Save(view,Path.Combine(output,"incoming-call.png"));
            manager.Calls.End(); Pump();
            Check(view.Visibility==Visibility.Collapsed,"Decline closes call immediately");
            // A stale connected update must not restart capture after local hangup.
            manager.Calls.Update(call with { phase="connected" });
            Check(typeof(CallController).GetField("_media",BindingFlags.NonPublic|BindingFlags.Instance)!.GetValue(manager.Calls)==null,"Hangup blocks stale capture");
            manager.Calls.Update(call with { callId="second-call",phase="ended",endReason="Call declined" });Pump();
            Check(((TextBlock)view.FindName("Status")).Text=="Call declined","Remote end reason");
            Check(((Button)view.FindName("End")).Content?.ToString()=="Done","Dismiss ended call");
            manager.Calls.Update(null);Pump();
            Check(view.Visibility==Visibility.Collapsed,"Logout removes call UI");
            Console.WriteLine("PASS: WPF incoming, decline, stale-state device privacy, ended, and logout");
            return 0;
        }
        catch(Exception e) {Console.Error.WriteLine(e);return 1;}
        finally {window.Close();manager.Shutdown();}
    }
    private static void Check(bool condition,string message) {if(!condition) throw new Exception(message);}
    private static void Pump()
    {
        var frame=new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.Background,new Action(()=>frame.Continue=false));
        Dispatcher.PushFrame(frame);
    }
    private static void Save(FrameworkElement view,string path)
    {
        view.UpdateLayout();
        var bitmap=new RenderTargetBitmap((int)view.ActualWidth,(int)view.ActualHeight,96,96,PixelFormats.Pbgra32);
        bitmap.Render(view);
        var png=new PngBitmapEncoder();png.Frames.Add(BitmapFrame.Create(bitmap));
        using var stream=File.Create(path);png.Save(stream);
    }
    private sealed class SilentNotifications : IDesktopNotificationPoster {public void Post(string title,string body) {}}
}
