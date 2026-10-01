using System;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class Program
{
    [STAThread]
    private static int Main(string[] args)
    {
        var output = Path.GetFullPath(args.Length > 0 ? args[0] : "work/direct-files-ui");
        Directory.CreateDirectory(output);
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_RUN_ID", Guid.NewGuid().ToString());
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_DATA_DIR", Path.Combine(output, "data"));
        var app = new App();
        app.InitializeComponent();
        var window = new Window { Title = "Direct files", Width = 560, Height = 620,
            Background = (Brush)app.FindResource("Background") };
        try
        {
            var report = Native.RunDirectFileTransferSmoke(output);
            File.WriteAllText(Path.Combine(output, "native-transfer.json"), report);
            using var evidence = JsonDocument.Parse(report);
            Check(evidence.RootElement.GetProperty("ok").GetBoolean(), report);
            Check(evidence.RootElement.GetProperty("files").GetArrayLength() == 3, "Native FIPS-TCP receives all three files");
            Check(evidence.RootElement.GetProperty("bytes_before_accept").GetInt64() == 0, "Native transport waits for acceptance");
            Check(Directory.GetDirectories(output, "iris-direct-files-smoke-*").Length == 0, "Native fixture removes temporary files");
            Console.WriteLine(report);
            var composer = new ComposerBar { DirectSendAllowed = true };
            composer.AddAttachments(new[] { "first.txt", "second.txt" });
            var mode = (CheckBox)composer.FindName("DirectMode");
            Check(mode.Visibility == Visibility.Visible && !composer.SendDirectly, "Files are reviewed before selecting direct send");
            mode.IsChecked = true;
            string[]? sent = null;
            composer.Submitted += (_, files) => { Check(composer.SendDirectly, "Direct choice reaches submit handler"); sent = new System.Collections.Generic.List<string>(files).ToArray(); };
            ((Button)composer.FindName("SendButton")).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
            Check(sent?.Length == 2 && !composer.SendDirectly, "Multiple files submitted and choice cleared");
            composer.DirectSendAllowed = false;
            composer.AddAttachments(new[] { "group.txt" });
            Check(mode.Visibility == Visibility.Collapsed, "Direct file mode excluded from groups");
            composer.Clear();

            var transfer = new DirectFileTransferSnapshot(
                id: "native-ui", files: new[] {
                    new DirectFileSnapshot(filename: "first.txt", sizeBytes: 10, localPath: null),
                    new DirectFileSnapshot(filename: "second.txt", sizeBytes: 20, localPath: null),
                }, status: DirectFileTransferStatus.Offered, isSender: false,
                transferredBytes: 0, totalBytes: 30, error: null);
            var receiving = new DirectFileTransferCard("self-chat", transfer);
            Check(Has(receiving, "chatDirectTransferAccept-native-ui"), "Receiving own device can accept");
            Check(Has(receiving, "chatDirectTransferDecline-native-ui"), "Receiving device can decline");
            Check(!Has(receiving, "chatDirectTransferOpen-native-ui"), "No file opens before acceptance");
            var sending = new DirectFileTransferCard("self-chat", transfer with { isSender = true });
            Check(!Has(sending, "chatDirectTransferAccept-native-ui") && Has(sending, "chatDirectTransferCancel-native-ui"), "Sender cancels rather than accepts");
            var progress = new DirectFileTransferCard("self-chat", transfer with {
                status = DirectFileTransferStatus.Transferring, transferredBytes = 15,
            });
            Check(progress.Children.OfType<ProgressBar>().Single().Value == 0.5, "Native progress reflects received bytes");
            Check(Has(progress, "chatDirectTransferCancel-native-ui") && !Has(progress, "chatDirectTransferAccept-native-ui"), "Active transfer can cancel and cannot accept again");
            var complete = new DirectFileTransferCard("self-chat", transfer with {
                status = DirectFileTransferStatus.Completed,
                files = new[] { transfer.files[0] with { localPath = Path.Combine(output, "first.txt") }, transfer.files[1] },
            });
            Check(Has(complete, "chatDirectTransferOpen-native-ui") && !Has(complete, "chatDirectTransferCancel-native-ui"), "Completed local files can open");
            var column = new StackPanel { Margin = new Thickness(20) };
            foreach (var card in new[] { receiving, sending, complete }) column.Children.Add(new Border {
                Child = card, Background = (Brush)app.FindResource("BubbleTheirs"),
                CornerRadius = new CornerRadius(14), Padding = new Thickness(12, 8, 12, 8),
                Margin = new Thickness(0, 0, 0, 8),
            });
            window.Content = column; window.Show(); Pump();
            Save(window, Path.Combine(output, "direct-file-states.png"));
            Console.WriteLine("PASS: WPF direct multi-file review, submit routing, own-device acceptance, progress/completion controls");
            return 0;
        }
        catch (Exception error) { Console.Error.WriteLine(error); return 1; }
        finally { window.Close(); }
    }

    private static bool Has(DependencyObject root, string id)
    {
        if (AutomationProperties.GetAutomationId(root) == id) return true;
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
            if (Has(VisualTreeHelper.GetChild(root, index), id)) return true;
        if (root is Panel panel) foreach (UIElement child in panel.Children) if (Has(child, id)) return true;
        return false;
    }
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); }
    private static void Pump()
    {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.Background, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }
    private static void Save(FrameworkElement element, string path)
    {
        element.UpdateLayout();
        var bitmap = new RenderTargetBitmap((int)element.ActualWidth, (int)element.ActualHeight, 96, 96, PixelFormats.Pbgra32);
        bitmap.Render(element);
        var png = new PngBitmapEncoder(); png.Frames.Add(BitmapFrame.Create(bitmap));
        using var file = File.Create(path); png.Save(file);
    }
}
