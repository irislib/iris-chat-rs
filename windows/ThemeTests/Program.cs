using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Markup;
using System.Xml.Linq;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using Microsoft.Win32;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;
using IrisChat.Views;

internal static class Program
{
    private static readonly MethodInfo Palette = typeof(App).GetMethod("ApplyPalette", BindingFlags.NonPublic | BindingFlags.Static)!;
    private static int MainResult;

    [STAThread]
    private static int Main(string[] args)
    {
        var output = Path.GetFullPath(args.Length > 0 ? args[0] : "work/themes");
        Directory.CreateDirectory(output);
        var runId = Guid.NewGuid().ToString("N");
        var data = Path.Combine(output, "data-" + runId);
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_RUN_ID", runId);
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_DATA_DIR", data);
        Environment.SetEnvironmentVariable("IRIS_DEMO_RELAYS", "ws://127.0.0.1:9");
        Environment.SetEnvironmentVariable("IRIS_FIPS_WEBSOCKET_SEED_URLS", "");
        var app = new TestApp { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        XNamespace xaml = "http://schemas.microsoft.com/winfx/2006/xaml/presentation";
        var resources = XDocument.Load(Path.Combine(AppContext.BaseDirectory, "App.xaml"))
            .Descendants(xaml + "ResourceDictionary").Single();
        resources.SetAttributeValue(XNamespace.Xmlns + "x", "http://schemas.microsoft.com/winfx/2006/xaml");
        app.Resources = (ResourceDictionary)XamlReader.Parse(resources.ToString()
            .Replace("clr-namespace:IrisChat", "clr-namespace:IrisChat;assembly=IrisChat"));
        var manager = new AppManager(data, new SilentNotifications(), Path.Combine(data, "secret.json"));
        typeof(App).GetProperty(nameof(App.Manager))!.SetValue(app, manager);
        var window = new Window { Width = 1366, Height = 900, Title = "Iris Chat theme checks",
            Background = (Brush)app.FindResource("Background"), Foreground = (Brush)app.FindResource("TextPrimary") };
        try
        {
            manager.CreateAccount("Theme test");
            PumpUntil(() => manager.Account != null);
            manager.CreateChat("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798");
            PumpUntil(() => manager.CurrentChat != null);
            var originalBrushes = new[] { "TextPrimary", "TextMuted", "Panel", "Background", "Danger" }
                .ToDictionary(key => key, key => (SolidColorBrush)app.FindResource(key));
            // Materialize styles before changing palettes: this is the startup
            // ordering that used to freeze white text and the dark input fill.
            var settings = new SettingsView();
            window.Content = settings;
            window.Show(); Pump();
            foreach (var light in new[] { true, false, true })
            {
                SetPalette(light); Pump();
                var theme = light ? "light" : "dark";
                foreach (var pair in originalBrushes)
                    Check(ReferenceEquals(pair.Value, app.FindResource(pair.Key)) && !pair.Value.IsFrozen,
                        $"{pair.Key} stays live after {theme} theme selection");
                Check(originalBrushes["TextPrimary"].Color == (Color)ColorConverter.ConvertFromString(light ? "#FF0F1419" : "#FFFFFFFF"),
                    $"{theme} primary text follows Windows palette");
                var menus = Descendants<ListBoxItem>(settings).Where(item => item.Tag is string).ToArray();
                Check(menus.Length == 13, "Every settings page is included");
                foreach (var menu in menus)
                {
                    menu.IsSelected = true; Pump();
                    Contrast.CheckTree(window, $"{theme} settings {menu.Tag}");
                    Save(window, Path.Combine(output, $"{theme}-settings-{menu.Tag}.png"));
                }
                Console.WriteLine($"PASS: {theme} settings navigation and all 13 pages");
            }
            foreach (var light in new[] { true, false })
            {
                SetPalette(light); Pump();
                var theme = light ? "light" : "dark";
                var shell = new DesktopShell(manager, manager.State.router.defaultScreen);
                Show(window, shell); Contrast.CheckTree(window, theme + " desktop sidebar");
                Save(window, Path.Combine(output, theme + "-sidebar.png"));
                foreach (var view in new FrameworkElement[] { new WelcomeView(), new CreateAccountView(),
                    new RestoreAccountView(), new NewChatView(), new NewGroupView(), new CreateInviteView(),
                    new JoinInviteView(), new DeviceRosterView(), new NearbyView(),
                    new ChatView { ChatId = manager.CurrentChat!.chatId } })
                {
                    Show(window, view); Contrast.CheckTree(window, theme + " " + view.GetType().Name);
                    Save(window, Path.Combine(output, theme + "-" + view.GetType().Name + ".png"));
                }
                Show(window, MessageChecks.Build(manager));
                Contrast.CheckTree(window, theme + " messages, audio and file transfers");
                Save(window, Path.Combine(output, theme + "-messages.png"));
                SetPalette(!light); Pump();
                Contrast.CheckTree(window, theme + " messages after live theme change");
                SetPalette(light); Pump();
                Contrast.CheckTree(window, theme + " messages after return theme change");
                Console.WriteLine($"PASS: {theme} sidebar, onboarding, new chat/group, invites, devices, message/attachment controls and live switching");
            }
            manager.OpenChat(manager.CurrentChat!.chatId);
            PumpUntil(() => manager.ActiveScreen is Screen.Chat);
            manager.Push(new Screen.Settings());
            PumpUntil(() => manager.ActiveScreen is Screen.Settings);
            var main = new MainWindow(data) { Width = 1366, Height = 900 };
            try
            {
                main.Show(); Pump();
                SettingsChecks.Run(main, manager, output);
            }
            finally { main.Close(); }
            var systemLight = (bool)typeof(App).GetMethod("UsesWindowsLightAppTheme", BindingFlags.NonPublic | BindingFlags.Static)!.Invoke(null, null)!;
            SetPalette(!systemLight);
            typeof(App).GetMethod("OnUserPreferenceChanged", BindingFlags.NonPublic | BindingFlags.Instance)!
                .Invoke(app, new object[] { app, new UserPreferenceChangedEventArgs(UserPreferenceCategory.General) });
            Pump();
            Check(originalBrushes["TextPrimary"].Color == (Color)ColorConverter.ConvertFromString(systemLight ? "#FF0F1419" : "#FFFFFFFF"),
                "Windows preference notifications re-read and apply the system app theme");
            Console.WriteLine($"PASS: Windows preference notification applies the current {(systemLight ? "light" : "dark")} app theme");
            Console.WriteLine($"PASS: {Contrast.Count} rendered text/control contrast checks, all >= 4.5:1 (icons >= 3:1)");
        }
        catch (Exception error)
        {
            Console.Error.WriteLine(error); MainResult = 1;
            Save(window, Path.Combine(output, "failure.png"));
        }
        finally { window.Close(); manager.Shutdown(); }
        return MainResult;
    }

    internal static void SetPalette(bool light) => Palette.Invoke(null, new object[] { light });
    internal static void Check(bool value, string context) { if (!value) throw new Exception(context); }
    internal static IEnumerable<T> Descendants<T>(DependencyObject root) where T : DependencyObject
    {
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
        {
            var child = VisualTreeHelper.GetChild(root, i);
            if (child is T match) yield return match;
            foreach (var nested in Descendants<T>(child)) yield return nested;
        }
    }
    internal static void Show(Window window, object content) { window.Content = content; Pump(); }
    internal static void Pump()
    {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
        foreach (Window window in Application.Current.Windows) window.UpdateLayout();
    }
    internal static void PumpUntil(Func<bool> predicate)
    {
        var deadline = DateTime.UtcNow.AddSeconds(30);
        while (!predicate()) { if (DateTime.UtcNow > deadline) throw new TimeoutException(); Pump(); }
    }
    internal static void Save(Window window, string path)
    {
        var content = (FrameworkElement)window.Content;
        var bitmap = new RenderTargetBitmap((int)Math.Ceiling(content.ActualWidth), (int)Math.Ceiling(content.ActualHeight), 96, 96, PixelFormats.Pbgra32);
        bitmap.Render(content);
        var png = new PngBitmapEncoder(); png.Frames.Add(BitmapFrame.Create(bitmap));
        using var stream = File.Create(path); png.Save(stream);
    }
    private sealed class TestApp : App
    {
        protected override void OnStartup(StartupEventArgs e) { }
        protected override void OnExit(ExitEventArgs e) { }
    }
    private sealed class SilentNotifications : IDesktopNotificationPoster
    {
        public void Post(string title, string body, DesktopNotificationTarget target) { }
        public void Clear() { }
    }
}
