using System;
using System.Collections.Generic;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using IrisChat.Chrome;

internal static class Program
{
    [STAThread]
    private static int Main(string[] args)
    {
        var app = new Application();
        var list = new ItemsControl();
        var scroll = new ScrollViewer { Content = list };
        var selection = new MessageTextSelection(scroll, list);
        var window = new Window { Content = scroll, Width = 460, Height = 300,
            Left = -3000, ShowActivated = false, ShowInTaskbar = false };
        var bodies = new List<MessageSelectionTextBox>();
        int checks = 0;
        void Check(bool pass, string name) { if (!pass) throw new Exception(name); checks++; }
        try
        {
            foreach (var text in new[] { "First message", "Middle 👩🏽‍💻 message\nsecond line", "Last message" })
            {
                var body = new MessageSelectionTextBox { IsReadOnly = true, IsDocumentEnabled = true,
                    Background = Brushes.Transparent, BorderThickness = new Thickness(0), Padding = new Thickness(0) };
                body.SetBodyDocument(text, new FlowDocument(new Paragraph(new Run(text))) { PagePadding = new Thickness(0) });
                var row = new Border { Child = body, Padding = new Thickness(12), Margin = new Thickness(8),
                    Background = Brushes.LightBlue, CornerRadius = new CornerRadius(14), Width = 300,
                    HorizontalAlignment = bodies.Count == 1 ? HorizontalAlignment.Right : HorizontalAlignment.Left };
                NameScope.SetNameScope(row, new NameScope()); row.RegisterName("BodyText", body);
                bodies.Add(body); list.Items.Add(row);
            }
            window.Show(); window.UpdateLayout();
            app.Dispatcher.Invoke(() => { }, DispatcherPriority.ApplicationIdle);
            TextPointer At(int row, int offset) => ((Run)((Paragraph)bodies[row].Document.Blocks.FirstBlock!).Inlines.FirstInline!)
                .ContentStart.GetPositionAtOffset(offset)!;
            selection.Select(bodies, bodies[0], At(0, 6), bodies[2], At(2, 4));
            const string expected = "message\nMiddle 👩🏽‍💻 message\nsecond line\nLast";
            Check(selection.SelectedText == expected, "Forward selection: " + selection.SelectedText.Replace("\r", "[CR]").Replace("\n", "[LF]"));
            selection.Select(bodies, bodies[2], At(2, 4), bodies[0], At(0, 6));
            Check(selection.SelectedText == expected, "Reverse drag keeps conversation order");
            bodies[0].SetBodyDocument("First message", new FlowDocument(new Paragraph(new Run("First message"))));
            Check(selection.SelectedText == expected, "Delivery/footer updates preserve selection");
            Check(bodies[0].IsInactiveSelectionHighlightEnabled && bodies[1].Selection.Text.Length > 0,
                "All message bodies retain visible native highlighting");
            Check(bodies.TrueForAll(view => AdornerLayer.GetAdornerLayer(view)?.GetAdorners(view)?.Length > 0),
                "Previously unfocused messages also have a visible selection highlight");
            window.Activate();
            bodies[0].Focus();
            window.UpdateLayout();
            app.Dispatcher.Invoke(() => { }, DispatcherPriority.ApplicationIdle);
            var image = new RenderTargetBitmap((int)window.ActualWidth, (int)window.ActualHeight, 96, 96, PixelFormats.Pbgra32);
            image.Render(window);
            if (args.Length > 0)
            {
                Directory.CreateDirectory(args[0]);
                var png = new PngBitmapEncoder(); png.Frames.Add(BitmapFrame.Create(image));
                using var output = File.Create(Path.Combine(args[0], "windows-selection.png")); png.Save(output);
            }
            selection.Clear();
            Check(selection.SelectedText == "" && bodies[1].Selection.IsEmpty, "Escape/reset clears every bubble");
            selection.Select(bodies, bodies[0], At(0, 6), bodies[2], At(2, 4));
            bodies[1].Document = new FlowDocument(new Paragraph(new Run("Edited")));
            Check(selection.SelectedText == "", "Editing selected text clears stale selection");
            selection.Select(bodies, bodies[0], At(0, 6), bodies[2], At(2, 4));
            list.Items.RemoveAt(2); selection.Reconcile();
            Check(selection.SelectedText == "", "Removing a selected message clears the range");
            Console.WriteLine($"PASS: {checks} native WPF selection checks");
            return 0;
        }
        catch (Exception error) { Console.Error.WriteLine(error); return 1; }
        finally { window.Close(); app.Shutdown(); }
    }
}
