using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
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
        if (args.Length == 2 && args[0] == "--clipboard-child") return Run(args[1]);
        var output = Path.GetFullPath(args.Length > 0 ? args[0] : "work/clipboard-ui");
        Directory.CreateDirectory(output);
        var start = new ProcessStartInfo(Environment.ProcessPath!) { UseShellExecute = false };
        start.ArgumentList.Add("--clipboard-child");
        start.ArgumentList.Add(output);
        using var child = Process.Start(start)!;
        if (!child.WaitForExit(180_000)) { child.Kill(entireProcessTree: true); return 1; }
        if (child.ExitCode != 0) return child.ExitCode;
        foreach (var name in File.ReadAllLines(Path.Combine(output, "generated-file-names.txt")))
            Check(!File.Exists(Path.Combine(Path.GetTempPath(), name)), "process exit removes submitted clipboard PNGs");
        Console.WriteLine("PASS: generated clipboard sources removed after native process exit");
        return 0;
    }

    private static int Run(string output)
    {
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_RUN_ID", Guid.NewGuid().ToString());
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_DATA_DIR", Path.Combine(output, "data"));
        var originalClipboard = Clipboard.GetDataObject();
        var app = new App();
        app.InitializeComponent();
        var composer = new ComposerBar { DirectSendAllowed = true,
            AttachmentPasteScope = () => new AttachmentPasteDestination("test-account", "chat-a") };
        var window = new Window { Title = "Clipboard attachments", Width = 620, Height = 280, Content = composer };
        var input = (TextBox)composer.FindName("Input");
        var direct = (CheckBox)composer.FindName("DirectMode");
        var submitted = 0;
        IList<string>? sent = null;
        composer.Submitted += (caption, files) =>
        {
            submitted++;
            Check(composer.SendDirectly, "paste preserves direct-send choice through manual submit");
            Check(caption == "Unsent caption plain text", "caption reaches submit unchanged");
            Check(files.All(File.Exists), "all submitted sources readable");
            sent = files;
        };
        try
        {
            window.Show(); Pump();
            input.Text = "Unsent caption "; input.CaretIndex = input.Text.Length;
            var first = Path.Combine(output, "original.png");
            var second = Path.Combine(output, "document.pdf");
            File.WriteAllBytes(first, new byte[] { 0, 1, 255, 42 });
            File.WriteAllText(second, "original document bytes");
            var pixels = new byte[] { 0, 0, 255, 255, 0, 255, 0, 255 };
            var bitmap = BitmapSource.Create(2, 1, 96, 96, PixelFormats.Bgra32, null, pixels, 8);
            var copied = new DataObject();
            copied.SetData(DataFormats.FileDrop, new[] { first, second });
            copied.SetData(DataFormats.Bitmap, bitmap);
            copied.SetText("file manager text must not replace caption");
            Clipboard.SetDataObject(copied);
            Paste(input);
            Check(composer.StagedFilePaths.SequenceEqual(new[] { first, second }), "all copied files win over duplicate bitmap");
            Check(File.ReadAllBytes(first).SequenceEqual(new byte[] { 0, 1, 255, 42 }), "original bytes preserved");
            direct.IsChecked = true;
            Clipboard.SetImage(bitmap);
            Paste(input);
            var image = composer.StagedFilePaths.Last();
            Check(composer.StagedFilePaths.Count == 3 && Path.GetExtension(image) == ".png", "screenshot staged as a PNG");
            using (var stream = File.OpenRead(image))
            {
                var decoded = BitmapFrame.Create(stream, BitmapCreateOptions.PreservePixelFormat, BitmapCacheOption.OnLoad);
                var rgba = new FormatConvertedBitmap(decoded, PixelFormats.Bgra32, null, 0);
                var actual = new byte[8]; rgba.CopyPixels(actual, 8, 0);
                Check(actual.SequenceEqual(pixels), "PNG contains exact screenshot pixels");
            }
            Check(input.Text == "Unsent caption " && composer.SendDirectly && submitted == 0, "paste preserves caption/mode and never sends");
            Clipboard.SetText("plain text"); input.CaretIndex = input.Text.Length;
            Paste(input);
            Check(input.Text == "Unsent caption plain text", "ordinary native text paste unchanged");
            Pump(); Save(window, Path.Combine(output, "windows-clipboard-draft.png"));
            var remove = Descendants(composer).OfType<Button>().Single(button => Equals(button.Tag, image));
            remove.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
            Check(!File.Exists(image) && first != image && File.Exists(first), "remove cleans generated file but never originals");

            composer.AttachmentPasteScope = () => null;
            Clipboard.SetImage(bitmap);
            Check(!ApplicationCommands.Paste.CanExecute(null, input), "busy/blocked/unavailable destination disables image paste");
            Check(composer.StagedFilePaths.Count == 2, "denied paste does not stage");
            var scopeReads = 0;
            composer.AttachmentPasteScope = () => new AttachmentPasteDestination("test-account", ++scopeReads == 1 ? "chat-a" : "chat-b");
            ApplicationCommands.Paste.Execute(null, input);
            Check(scopeReads == 2 && composer.StagedFilePaths.Count == 2, "scope change during clipboard rendering discards paste");
            composer.AttachmentPasteScope = () => new AttachmentPasteDestination("test-account", "chat-a");
            Clipboard.SetData(DataFormats.FileDrop, new[] { first, output });
            Paste(input);
            Check(composer.StagedFilePaths.Count == 2, "mixed directory selection rejected atomically");

            Clipboard.SetImage(bitmap); Paste(input);
            image = composer.StagedFilePaths.Last();
            ((Button)composer.FindName("SendButton")).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
            Check(submitted == 1 && sent?.Count == 3 && composer.StagedFilePaths.Count == 0, "only manual Send submits files");
            Check(File.Exists(image), "generated source survives queued asynchronous ingestion");
            File.WriteAllLines(Path.Combine(output, "generated-file-names.txt"), new[] { Path.GetFileName(image) });
            Clipboard.SetImage(bitmap); Paste(input);
            var cancelled = composer.StagedFilePaths.Single();
            composer.Clear();
            Check(!File.Exists(cancelled) && File.Exists(image), "clear cancels unsent draft without deleting submitted source");
            var large = string.Concat(Enumerable.Repeat("A long pasted note, with Unicode 世界.\r\n", 4096));
            var changes = 0;
            TextChangedEventHandler changed = (_, _) => changes++;
            input.TextChanged += changed;
            Clipboard.SetText(large);
            input.Focus(); Pump();
            using var layoutProbe = new PasteLayoutProbe();
            var elapsed = Stopwatch.StartNew();
            Check(ApplicationCommands.Paste.CanExecute(null, input), "native Paste enabled");
            var canExecuteMs = elapsed.Elapsed.TotalMilliseconds;
            ApplicationCommands.Paste.Execute(null, input);
            var executeCompleteMs = elapsed.Elapsed.TotalMilliseconds;
            Pump();
            var firstDrainCompleteMs = elapsed.Elapsed.TotalMilliseconds;
            window.UpdateLayout();
            var layoutCompleteMs = elapsed.Elapsed.TotalMilliseconds;
            Pump();
            var finalDrainCompleteMs = elapsed.Elapsed.TotalMilliseconds;
            var pastedCaret = input.GetRectFromCharacterIndex(input.CaretIndex);
            window.UpdateLayout(); Pump();
            pastedCaret = input.GetRectFromCharacterIndex(input.CaretIndex);
            elapsed.Stop();
            layoutProbe.Finish();
            var pasteMs = elapsed.Elapsed.TotalMilliseconds;
            var pasteChanges = changes;
            Check(input.Text == large && changes == 1, "large plain text is one native editor change");
            Check(input.VerticalScrollBarVisibility == ScrollBarVisibility.Visible && ComposerViewport.Contains(input, pastedCaret),
                "production bulk paste completes layout with its caret visible");
            Check(composer.StagedFilePaths.Count == 0 && submitted == 1, "large text does not stage or send files");
            input.CaretIndex = input.Text.Length;
            elapsed.Restart();
            input.SelectedText = "!";
            var editInsertMs = elapsed.Elapsed.TotalMilliseconds;
            window.UpdateLayout();
            var editLayoutCompleteMs = elapsed.Elapsed.TotalMilliseconds;
            Pump();
            elapsed.Stop();
            var editMs = elapsed.Elapsed.TotalMilliseconds;
            var editChanges = changes - pasteChanges;
            input.TextChanged -= changed;
            const double freezeBudgetMs = 5_000;
            var timings = JsonSerializer.Serialize(new
            {
                platform = "windows", utf16_code_units = large.Length,
                utf8_bytes = Encoding.UTF8.GetByteCount(large),
                paste_ms = pasteMs, subsequent_edit_ms = editMs,
                first_render_ms = layoutProbe.FirstRenderMs,
                longest_dispatcher_gap_ms = layoutProbe.LongestDispatcherGapMs,
                paste_phases_ms = new
                {
                    can_execute = canExecuteMs,
                    execute = executeCompleteMs - canExecuteMs,
                    first_dispatcher_drain = firstDrainCompleteMs - executeCompleteMs,
                    layout = layoutCompleteMs - firstDrainCompleteMs,
                    final_dispatcher_drain = finalDrainCompleteMs - layoutCompleteMs,
                    caret_geometry_layout_drain = pasteMs - finalDrainCompleteMs,
                },
                subsequent_edit_phases_ms = new
                {
                    insert = editInsertMs,
                    layout = editLayoutCompleteMs - editInsertMs,
                    dispatcher_drain = editMs - editLayoutCompleteMs,
                },
                paste_change_events = pasteChanges, subsequent_edit_change_events = editChanges,
                freeze_budget_ms = freezeBudgetMs,
                boundary = "Clipboard already populated; native Paste command through forced window layout, background-priority dispatcher drain, and refreshed end-caret geometry. Subsequent edit uses native SelectedText insertion through the same layout/drain.",
            }, new JsonSerializerOptions { WriteIndented = true });
            File.WriteAllText(Path.Combine(output, "windows-text-paste-timings.json"), timings);
            Console.WriteLine($"TIMING: Windows large text paste {pasteMs:F1} ms; subsequent edit {editMs:F1} ms; events {pasteChanges}/{editChanges}");
            Check(input.Text == large + "!" && editChanges == 1, "subsequent native edit completes once after large paste");
            Check(pasteMs < freezeBudgetMs && editMs < freezeBudgetMs, "large paste and subsequent edit stay within the 5 s freeze budget");
            Check(executeCompleteMs - canExecuteMs < 500, "large native paste command returns without full-document synchronous layout");
            Save(window, Path.Combine(output, "windows-large-text-paste.png"));
            LargeTextPasteTests.Verify(composer, Pump, output);
            RestoredDraftLayoutTests.Verify(window, composer, large, Pump, output);
            PasteLayoutComparison.Verify(window, composer, large, Pump, output);
            composer.Clear();
            Clipboard.SetImage(bitmap); Paste(input);
            cancelled = composer.StagedFilePaths.Single();
            window.Content = null; Pump();
            Check(!File.Exists(cancelled), "unloading the old chat cleans its draft");
            ReentrantPasteTests.Verify(window, bitmap, first);
            Console.WriteLine("PASS: WPF native clipboard paste, multiple originals, PNG pixels, direct draft, large text single-edit, no auto-send, stale/blocked destination and generated-file cleanup");
            return 0;
        }
        catch (Exception error) { Console.Error.WriteLine(error); return 1; }
        finally
        {
            composer.Clear(); window.Close();
            if (originalClipboard != null) Clipboard.SetDataObject(originalClipboard, true);
            else Clipboard.Clear();
        }
    }

    private static void Paste(TextBox input)
    {
        Check(ApplicationCommands.Paste.CanExecute(null, input), "native Paste enabled");
        ApplicationCommands.Paste.Execute(null, input); Pump();
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject root)
    {
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
        {
            var child = VisualTreeHelper.GetChild(root, i); yield return child;
            foreach (var nested in Descendants(child)) yield return nested;
        }
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
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
