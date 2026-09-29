using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using IrisChat;

internal static class AttachmentDropTests
{
    internal static void Run()
    {
        var directory = Path.Combine(Path.GetTempPath(), Guid.NewGuid().ToString());
        Directory.CreateDirectory(directory);
        try
        {
            var first = Path.Combine(directory, "first.txt");
            var second = Path.Combine(directory, "second.pdf");
            File.WriteAllText(first, "one"); File.WriteAllText(second, "two");
            var input = new TextBox { Text = "Unsent caption" };
            var surface = new Border { Child = input };
            var allowed = true;
            var highlighted = false;
            var staged = new List<string>();
            _ = new AttachmentDropTarget(surface, () => allowed,
                paths => staged.AddRange(paths), active => highlighted = active);
            DragEventArgs Raise(IDataObject data, RoutedEvent route)
            {
                // WPF creates these args internally for OLE drags. Invoke that
                // constructor, then exercise the actual routed event handlers.
                var args = (DragEventArgs)Activator.CreateInstance(typeof(DragEventArgs),
                    BindingFlags.Instance | BindingFlags.NonPublic, null,
                    new object[] { data, DragDropKeyStates.None, DragDropEffects.Copy, input, new Point(4, 4) }, null)!;
                args.RoutedEvent = route;
                input.RaiseEvent(args);
                return args;
            }
            var files = new DataObject(DataFormats.FileDrop, new[] { first, second });
            var hover = Raise(files, DragDrop.PreviewDragOverEvent);
            Check(highlighted && hover.Effects == DragDropEffects.Copy, "valid file drag highlighted");
            Check(staged.Count == 0, "hover does not stage or send");
            var drop = Raise(files, DragDrop.PreviewDropEvent);
            Check(drop.Handled && !highlighted && staged.Count == 2 && staged[0] == first && staged[1] == second,
                "production routed drop stages ordered files");
            Check(input.Text == "Unsent caption", "drop preserves caption instead of inserting paths");
            staged.Clear();
            allowed = false;
            Raise(files, DragDrop.PreviewDropEvent);
            Check(staged.Count == 0, "chat change/block/busy gate is checked at drop time");
            allowed = true;
            var folder = new DataObject(DataFormats.FileDrop, new[] { first, directory });
            Raise(folder, DragDrop.PreviewDragOverEvent);
            Check(!highlighted, "folder selection is not highlighted");
            Raise(folder, DragDrop.PreviewDropEvent);
            Check(staged.Count == 0, "mixed folder selection rejected atomically");
            var text = Raise(new DataObject(DataFormats.Text, "hello"), DragDrop.PreviewDragOverEvent);
            Check(!text.Handled && !highlighted, "ordinary text drag remains with editor");
            Console.WriteLine("PASS: file drop routed events, caption, eligibility, and directory rejection");
        }
        finally { Directory.Delete(directory, true); }
    }
    private static void Check(bool condition, string message)
    { if (!condition) throw new InvalidOperationException(message); }
}
