using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Windows;

namespace IrisChat;

/// Handles file drops before a child text box can insert file paths as text.
internal sealed class AttachmentDropTarget
{
    internal AttachmentDropTarget(FrameworkElement surface, Func<bool> canAttach,
        Action<IReadOnlyList<string>> stage, Action<bool> highlight)
    {
        surface.AllowDrop = true;
        surface.PreviewDragOver += (sender, e) =>
        {
            var accepted = canAttach() && TryGetFiles(e.Data, out _)
                && (e.AllowedEffects & DragDropEffects.Copy) != 0;
            e.Effects = accepted ? DragDropEffects.Copy : DragDropEffects.None;
            // Leave ordinary text drags to the editor.
            e.Handled = e.Data.GetDataPresent(DataFormats.FileDrop);
            highlight(accepted);
        };
        surface.PreviewDragLeave += (_, _) => highlight(false);
        surface.Unloaded += (_, _) => highlight(false);
        surface.PreviewDrop += (_, e) =>
        {
            highlight(false);
            if (!e.Data.GetDataPresent(DataFormats.FileDrop)) return;
            e.Handled = true;
            e.Effects = DragDropEffects.None;
            if (!canAttach() || (e.AllowedEffects & DragDropEffects.Copy) == 0 ||
                !TryGetFiles(e.Data, out var files)) return;
            stage(files);
            e.Effects = DragDropEffects.Copy;
        };
    }

    private static bool TryGetFiles(IDataObject data, out IReadOnlyList<string> files)
    {
        files = Array.Empty<string>();
        try
        {
            if (data.GetData(DataFormats.FileDrop) is not string[] paths || paths.Length == 0 ||
                paths.Any(path => !Path.IsPathFullyQualified(path) || !File.Exists(path))) return false;
            files = paths.Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
            return true;
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or
                                      System.Runtime.InteropServices.COMException or ArgumentException)
        {
            return false;
        }
    }
}
