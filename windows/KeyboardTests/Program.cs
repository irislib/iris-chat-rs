using System;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Input;
using System.Windows.Markup;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using System.Xml.Linq;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class Program
{
    private static readonly Type Navigation = typeof(ChatRow).Assembly.GetType("IrisChat.Chrome.KeyboardList")!;
    [STAThread]
    private static int Main(string[] args)
    {
        var output = Path.GetFullPath(args.Length > 0 ? args[0] : "work/keyboard");
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
        var window = new Window { Width = 500, Height = 470, Title = "Keyboard navigation",
            Background = (Brush)app.FindResource("Background") };
        try
        {
            manager.CreateAccount("Keyboard test"); PumpUntil(() => manager.Account != null);
            manager.CreateChat("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798");
            PumpUntil(() => manager.CurrentChat != null);
            var chat = manager.ChatList.First();
            var first = new ChatRow { Uid = "first", Chat = chat with { chatId = "first", displayName = "First chat" } };
            var second = new ChatRow { Chat = chat with { chatId = "second", displayName = "Second chat" } };
            var root = new StackPanel { Margin = new Thickness(16) };
            var search = new TextBox { Text = "Search" };
            var list = new ItemsControl();
            Navigation.GetMethod("Install")!.Invoke(null, new object[] { list });
            Check(!list.Focusable && !KeyboardNavigation.GetIsTabStop(list), "The list container is not a keyboard stop");
            list.Items.Add(new TextBlock { Text = "Pinned", Focusable = false }); list.Items.Add(first);
            list.Items.Add(new TextBlock { Text = "Chats", Focusable = false }); list.Items.Add(second);
            for (int i = 2; i < 140; i++) list.Items.Add(new ChatRow { Chat = chat with { chatId = $"chat-{i}", displayName = $"Synthetic chat {i}" } });
            var scroll = new ScrollViewer { Content = list, Height = 250, Focusable = false, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
            var after = new Button { Content = "Settings", Style = (Style)app.FindResource("GhostButton") };
            var composer = new ComposerBar();
            root.Children.Add(search); root.Children.Add(scroll); root.Children.Add(after); root.Children.Add(composer);
            window.Content = root; window.Show(); window.Activate(); Pump();
            search.Focus(); search.MoveFocus(new TraversalRequest(FocusNavigationDirection.Next));
            Check(first.IsKeyboardFocused, $"Tab enters the list at its first chat (actual: {Keyboard.FocusedElement?.GetType().Name})");
            int opened = 0; first.Activated += _ => opened++;
            var activeChat = manager.CurrentChat!.chatId;
            for (int i = 0; i < 80; i++) Key(first, System.Windows.Input.Key.Down, Keyboard.PreviewKeyDownEvent);
            Check(scroll.VerticalOffset > 2000 && first.IsKeyboardFocused && opened == 0
                && manager.CurrentChat.chatId == activeChat, "Long Down scroll preserves focus and active chat");
            for (int i = 0; i < 80; i++) Key(first, System.Windows.Input.Key.Up, Keyboard.PreviewKeyDownEvent);
            Check(scroll.VerticalOffset == 0 && first.IsKeyboardFocused, "Long Up scroll preserves focus");
            for (int i = 0; i < 80; i++) Key(first, System.Windows.Input.Key.Down, Keyboard.PreviewKeyDownEvent);
            Check(scroll.VerticalOffset > 2000 && first.IsKeyboardFocused, "Offscreen focused row keeps receiving arrows");
            var offset = scroll.VerticalOffset;
            using ((IDisposable)Navigation.GetMethod("PreserveFocus")!.Invoke(null, new object[] { list })!)
            {
                list.Items.Clear();
                first = new ChatRow { Chat = first.Chat, Uid = "first" };
                list.Items.Add(first);
                for (int i = 1; i < 140; i++) list.Items.Add(new ChatRow { Chat = chat with { chatId = $"chat-{i}", displayName = $"Synthetic chat {i}" } });
            }
            Pump(); Check(first.IsKeyboardFocused && Math.Abs(scroll.VerticalOffset - offset) < 2,
                "State refresh preserves offscreen focused identity and viewport");
            Key(first, System.Windows.Input.Key.Home, Keyboard.PreviewKeyDownEvent);
            Check(first.IsKeyboardFocused && scroll.VerticalOffset == 0, "Home scrolls only");
            Key(first, System.Windows.Input.Key.End, Keyboard.PreviewKeyDownEvent);
            Check(first.IsKeyboardFocused && Math.Abs(scroll.VerticalOffset - scroll.ScrollableHeight) < 2, "End scrolls only");
            first.Activated += _ => opened++;
            Key(first, System.Windows.Input.Key.Enter, Keyboard.KeyDownEvent);
            Key(first, System.Windows.Input.Key.Space, Keyboard.KeyDownEvent);
            Check(opened == 2, "Enter and Space each activate focused row once");
            first.MoveFocus(new TraversalRequest(FocusNavigationDirection.Next));
            Check(after.IsKeyboardFocused, "One Tab exits the complete chat list");
            after.MoveFocus(new TraversalRequest(FocusNavigationDirection.Previous));
            Check(first.IsKeyboardFocused, "Shift-Tab returns to the focused chat");
            composer.FocusInput();
            var input = (TextBox)composer.FindName("Input"); input.Text = "draft"; input.CaretIndex = 2;
            Check(input.AcceptsTab, "Composer accepts literal Tab");
            EditingCommands.TabForward.Execute(null, input);
            Check(input.Text == "dr\taft" && input.IsKeyboardFocused, "Tab inserts into the draft");
            input.Text = "draft";
            var arrow = Key(input, System.Windows.Input.Key.Up, Keyboard.PreviewKeyDownEvent);
            Check(!arrow.Handled && input.IsKeyboardFocused && input.Text == "draft", "List arrows do not intercept composer editing");
            using ((IDisposable)Navigation.GetMethod("PreserveFocus")!.Invoke(null, new object[] { list })!) { }
            Pump(); Check(input.IsKeyboardFocused, "Background sidebar refresh does not steal focus");
            string? sent = null; composer.Submitted += (text, _) => sent = text;
            Key(input, System.Windows.Input.Key.Enter, Keyboard.PreviewKeyDownEvent);
            Check(sent == "draft" && input.Text.Length == 0, "Composer Enter behavior is retained");
            first.Focus(); scroll.ScrollToTop(); window.UpdateLayout();
            var image = new RenderTargetBitmap((int)window.ActualWidth, (int)window.ActualHeight, 96, 96, PixelFormats.Pbgra32);
            image.Render(window); var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(image));
            using (var file = File.Create(Path.Combine(output, "windows-keyboard.png"))) encoder.Save(file);
            GroupingTests.Run();
            HistoryTests.Run(window, manager);
            Console.WriteLine("PASS: WPF Tab/Shift-Tab, arrows, Enter/Space, update focus and composer isolation");
            return 0;
        }
        catch (Exception error) { Console.Error.WriteLine(error); return 1; }
        finally { window.Close(); manager.Shutdown(); app.Shutdown(); }
    }
    private static KeyEventArgs Key(UIElement target, Key key, RoutedEvent routedEvent)
    {
        var e = new KeyEventArgs(Keyboard.PrimaryDevice, PresentationSource.FromVisual(target), 0, key) { RoutedEvent = routedEvent };
        target.RaiseEvent(e); Pump(); return e;
    }
    private static void Pump()
    {
        var frame = new DispatcherFrame(); Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,
            new Action(() => frame.Continue = false)); Dispatcher.PushFrame(frame);
    }
    private static void PumpUntil(Func<bool> ready)
    {
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (!ready()) { Check(DateTime.UtcNow < deadline, "Isolated account timed out"); Pump(); System.Threading.Thread.Sleep(5); }
    }
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); }
    private sealed class TestApp : App { protected override void OnStartup(StartupEventArgs e) { } }
    private sealed class SilentNotifications : IDesktopNotificationPoster
    {
        public void Post(string title, string body, DesktopNotificationTarget target) { }
        public void Clear() { }
    }
}
