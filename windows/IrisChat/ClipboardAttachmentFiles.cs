using System;
using System.Collections.Generic;
using System.IO;

namespace IrisChat;

/// Owns only PNGs created from clipboard pixels, never original copied files.
internal sealed class ClipboardAttachmentFiles : IDisposable
{
    private readonly HashSet<string> _drafts = new(StringComparer.OrdinalIgnoreCase);
    private static readonly HashSet<string> SessionFiles = new(StringComparer.OrdinalIgnoreCase);

    static ClipboardAttachmentFiles()
    {
        // Core reads queued sources asynchronously, including direct-send
        // preparation. Keep paths (not pixels) until app exit. Do not hold a
        // writable/DeleteOnClose handle: ordinary preview/cache readers need to
        // open these paths with their normal file-sharing flags.
        AppDomain.CurrentDomain.ProcessExit += (_, _) =>
        {
            foreach (var path in SessionFiles) Delete(path);
            SessionFiles.Clear();
        };
    }

    internal string CreatePng(Action<Stream> write)
    {
        var path = Path.Combine(Path.GetTempPath(), $"Pasted image-{Guid.NewGuid():N}.png");
        try
        {
            using (var stream = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                write(stream);
            _drafts.Add(path);
            SessionFiles.Add(path);
            return path;
        }
        catch { Delete(path); throw; }
    }

    internal void Remove(string path)
    {
        if (_drafts.Remove(path)) { Delete(path); SessionFiles.Remove(path); }
    }

    internal void HandOff(IEnumerable<string> paths)
    {
        foreach (var path in paths)
            _drafts.Remove(path);
    }

    public void Dispose()
    {
        foreach (var path in _drafts) { Delete(path); SessionFiles.Remove(path); }
        _drafts.Clear();
    }

    private static void Delete(string path)
    {
        try { File.Delete(path); }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException) { }
    }
}
