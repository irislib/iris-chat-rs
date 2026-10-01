using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media.Imaging;

namespace IrisChat;

/// Extends the editor's native Paste command; ordinary text remains WPF's job.
internal sealed class AttachmentPasteTarget
{
    internal AttachmentPasteTarget(TextBox input, Func<string?> scope,
        Action<IReadOnlyList<string>> stage, ClipboardAttachmentFiles files, Action failed)
    {
        CommandManager.AddPreviewCanExecuteHandler(input, (_, e) =>
        {
            if (e.Command != ApplicationCommands.Paste) return;
            try
            {
                if (!HasAttachment(Clipboard.GetDataObject())) return;
                e.CanExecute = input.IsLoaded && scope() != null;
                e.Handled = true; // TextBox otherwise disables image-only paste.
            }
            catch (Exception error) when (ClipboardError(error)) { }
        });
        CommandManager.AddPreviewExecutedHandler(input, (_, e) =>
        {
            if (e.Command != ApplicationCommands.Paste) return;
            string? generated = null;
            try
            {
                var data = Clipboard.GetDataObject();
                if (!HasAttachment(data)) return;
                e.Handled = true;
                var captured = scope();
                if (captured == null || !input.IsLoaded) return;
                IReadOnlyList<string> paths;
                if (data!.GetDataPresent(DataFormats.FileDrop))
                {
                    // FileDrop wins over a file manager's bitmap thumbnail.
                    if (data.GetData(DataFormats.FileDrop) is not string[] copied || copied.Length == 0 ||
                        copied.Any(path => !Path.IsPathFullyQualified(path) || !File.Exists(path))) return;
                    paths = copied.Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
                }
                else if (ReadImage(data) is BitmapSource bitmap)
                {
                    generated = files.CreatePng(stream =>
                    {
                        var encoder = new PngBitmapEncoder();
                        encoder.Frames.Add(BitmapFrame.Create(bitmap));
                        encoder.Save(stream);
                    });
                    paths = new[] { generated };
                }
                else return;
                // Clipboard providers may pump messages while rendering data.
                if (scope() != captured || !input.IsLoaded) return;
                stage(paths);
                generated = null; // Draft now owns the source; no auto-send.
            }
            catch (Exception error) when (ClipboardError(error)) { failed(); }
            finally { if (generated != null) files.Remove(generated); }
        });
    }

    private static bool HasAttachment(IDataObject? data) => data != null &&
        (data.GetDataPresent(DataFormats.FileDrop) || data.GetDataPresent("PNG") ||
         data.GetDataPresent(DataFormats.Bitmap));

    private static BitmapSource? ReadImage(IDataObject data)
    {
        // PNG preserves transparency that the standard clipboard bitmap can lose.
        if (data.GetDataPresent("PNG") && data.GetData("PNG") is Stream png)
        {
            if (png.CanSeek) png.Position = 0;
            return BitmapFrame.Create(png, BitmapCreateOptions.PreservePixelFormat, BitmapCacheOption.OnLoad);
        }
        return data.GetData(DataFormats.Bitmap) as BitmapSource;
    }

    private static bool ClipboardError(Exception error) => error is IOException or
        UnauthorizedAccessException or COMException or ArgumentException or
        NotSupportedException or InvalidOperationException;
}
