using System;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using System.Windows.Markup;
using System.Xml.Linq;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;
using IrisChat.Views;

internal static class Program
{
    private const string Peer = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    private static readonly MethodInfo Apply = typeof(AppManager).GetMethod("Apply", BindingFlags.Instance | BindingFlags.NonPublic)!;

    [STAThread]
    private static int Main(string[] args)
    {
        var output = Path.GetFullPath(args.Length > 0 ? args[0] : "work/nearby-avatar");
        Directory.CreateDirectory(output);
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_RUN_ID", Guid.NewGuid().ToString());
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_DATA_DIR", Path.Combine(output, "data"));
        Environment.SetEnvironmentVariable("IRIS_DEMO_RELAYS", "ws://127.0.0.1:9");
        Environment.SetEnvironmentVariable("IRIS_FIPS_WEBSOCKET_SEED_URLS", "");
        var app = new TestApp { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        XNamespace xaml = "http://schemas.microsoft.com/winfx/2006/xaml/presentation";
        var resources = XDocument.Load(Path.Combine(AppContext.BaseDirectory, "App.xaml")).Descendants(xaml + "ResourceDictionary").Single();
        resources.SetAttributeValue(XNamespace.Xmlns + "x", "http://schemas.microsoft.com/winfx/2006/xaml");
        app.Resources = (ResourceDictionary)XamlReader.Parse(resources.ToString());
        var manager = new AppManager(Path.Combine(output, "data"), new SilentNotifications(), Path.Combine(output, "secret.json"));
        typeof(App).GetProperty(nameof(App.Manager))!.SetValue(app, manager);
        var window = new Window { Title = "Nearby avatars", Width = 700, Height = 650,
            Background = (Brush)app.FindResource("Background") };
        try
        {
            manager.CreateAccount("Nearby test");
            PumpUntil(() => manager.Account != null);
            manager.CreateChat(Peer);
            PumpUntil(() => manager.CurrentChat?.chatId == Peer);
            var connection = new SocialConnectionSnapshot(SocialBadge.Following, 1, 0, "Following");
            var state = manager.State with {
                rev = manager.State.rev + 1,
                preferences = manager.Preferences with { nearbyEnabled = true, nearbyShowInChatList = false },
                currentChat = manager.CurrentChat! with { displayName = "Alex", socialConnection = connection },
            };
            Apply.Invoke(manager, new object[] { new AppUpdate.FullState(state) });
            var avatar = new Avatar { Label = "Alex", Size = 64, OwnerPubkeyHex = Peer, SocialConnection = connection };
            var own = new Avatar { Label = "You", Size = 48, OwnerPubkeyHex = manager.Account!.publicKeyHex };
            var unknown = new Avatar { Label = "Unknown", Size = 48, OwnerPubkeyHex = "other-person" };
            var group = new ChatRow { Chat = manager.ChatList.First(c => c.chatId == Peer) with { kind = ChatKind.Group } };
            var chat = new ChatView { ChatId = Peer, MinHeight = 360 };
            var column = new StackPanel { Margin = new Thickness(20) };
            column.Children.Add(chat);
            var examples = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 20, 0, 12) };
            foreach (var item in new[] { avatar, own, unknown }) { item.Margin = new Thickness(12); examples.Children.Add(item); }
            column.Children.Add(examples); column.Children.Add(group);
            window.Content = column; window.Show(); Pump();
            var mark = (Border)avatar.FindName("NearbyMark");
            var social = (Border)avatar.FindName("SocialMark");
            var header = (Avatar)chat.FindName("HeaderAvatar");
            Check(!avatar.IsNearby, "No badge before a matching nearby peer");
            var peer = new DesktopNearbyPeerSnapshot("device", "Alex", Peer.ToUpperInvariant(), null, null, 1);
            var nearby = new DesktopNearbySnapshot(true, "Nearby", new[] { peer,
                peer with { id = "own-device", ownerPubkeyHex = manager.Account.publicKeyHex } });
            Apply.Invoke(manager, new object[] { new AppUpdate.NearbyPeersChanged(nearby, Array.Empty<string>(), Array.Empty<string>()) });
            Check(avatar.IsNearby && header.IsNearby, "Existing avatar and chat header react to nearby updates");
            Check(!own.IsNearby && !unknown.IsNearby, "Own devices and unrelated people stay unbadged");
            Check(!((Avatar)group.FindName("AvatarView")).IsNearby, "Group avatar stays unbadged");
            Check(mark.Visibility == Visibility.Visible && social.Visibility == Visibility.Visible, "Nearby and verified marks coexist");
            Check(mark.HorizontalAlignment == HorizontalAlignment.Right && mark.VerticalAlignment == VerticalAlignment.Bottom,
                "Nearby mark anchors bottom-right");
            Check(social.VerticalAlignment == VerticalAlignment.Top, "Verified mark remains at top");
            Apply.Invoke(manager, new object[] { new AppUpdate.NearbyPeersChanged(nearby with { peers = Array.Empty<DesktopNearbyPeerSnapshot>() }, Array.Empty<string>(), Array.Empty<string>()) });
            Check(!avatar.IsNearby && !header.IsNearby, "Peer removal immediately hides both badges");
            Apply.Invoke(manager, new object[] { new AppUpdate.NearbyPeersChanged(nearby, Array.Empty<string>(), Array.Empty<string>()) });
            var disabled = manager.State with { rev = manager.State.rev + 1, preferences = manager.Preferences with { nearbyEnabled = false } };
            Apply.Invoke(manager, new object[] { new AppUpdate.FullState(disabled) });
            Check(!avatar.IsNearby && !header.IsNearby, "Turning nearby off clears badges");
            Apply.Invoke(manager, new object[] { new AppUpdate.FullState(disabled with {
                rev = disabled.rev + 1, preferences = disabled.preferences with { nearbyEnabled = true } }) });
            Apply.Invoke(manager, new object[] { new AppUpdate.NearbyPeersChanged(nearby with { visible = false }, Array.Empty<string>(), Array.Empty<string>()) });
            Check(!avatar.IsNearby, "Stopped discovery hides stale peer state");
            Apply.Invoke(manager, new object[] { new AppUpdate.NearbyPeersChanged(nearby, Array.Empty<string>(), Array.Empty<string>()) });
            window.UpdateLayout();
            var markPosition = mark.TranslatePoint(new Point(), avatar);
            var socialPosition = social.TranslatePoint(new Point(), avatar);
            Check(markPosition.Y >= socialPosition.Y + social.ActualHeight, $"Badge bounds do not overlap (nearbyY={markPosition.Y}, socialY={socialPosition.Y}, socialHeight={social.ActualHeight})");
            var headerMark = (Border)header.FindName("NearbyMark");
            var headerSocial = (Border)header.FindName("SocialMark");
            Check(headerMark.TranslatePoint(new Point(), header).Y >= headerSocial.TranslatePoint(new Point(), header).Y + headerSocial.ActualHeight,
                "Small chat header badges do not overlap");
            Save(window, Path.Combine(output, "windows-nearby-avatars.png"));
            Console.WriteLine("PASS: WPF nearby avatars/header, live removal, preferences, self/group exclusion and badge placement");
            return 0;
        }
        catch (Exception error) { Console.Error.WriteLine(error); return 1; }
        finally { window.Close(); manager.Shutdown(); }
    }

    private static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
    private static void Pump() {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.Background, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }
    private static void PumpUntil(Func<bool> ready) {
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (!ready()) { Pump(); if (DateTime.UtcNow >= deadline) throw new Exception("Core update timed out"); System.Threading.Thread.Sleep(2); }
    }
    private static void Save(FrameworkElement element, string path) {
        var bitmap = new RenderTargetBitmap((int)element.ActualWidth, (int)element.ActualHeight, 96, 96, PixelFormats.Pbgra32);
        bitmap.Render(element); var png = new PngBitmapEncoder(); png.Frames.Add(BitmapFrame.Create(bitmap));
        using var file = File.Create(path); png.Save(file);
    }
    // Application schedules startup on the dispatcher even without Run(). Keep this
    // fixture on its explicit isolated manager instead of opening the normal profile.
    private sealed class TestApp : App { protected override void OnStartup(StartupEventArgs e) { } }
    private sealed class SilentNotifications : IDesktopNotificationPoster {
        public void Post(string title, string body, DesktopNotificationTarget target) { }
        public void Clear() { }
    }
}
