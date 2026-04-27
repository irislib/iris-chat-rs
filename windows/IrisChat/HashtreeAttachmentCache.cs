using System;
using System.Collections.Concurrent;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using IrisChat.Bindings;

namespace IrisChat;

/// Disk-backed cache for blobs fetched via the Rust core's blossom resolver.
/// Mirrors the iOS AppManager's attachment cache behaviour:
///   - cap on disk usage with LRU eviction
///   - identical key (nhash) returns identical bytes so callers can be cheap
public sealed class HashtreeAttachmentCache
{
    private const long DefaultCacheLimitBytes = 128L * 1024L * 1024L;

    private readonly string _attachmentsRoot;
    private readonly string _downloadedDir;
    private readonly long _cacheLimitBytes;
    private readonly ConcurrentDictionary<string, Lazy<Task<byte[]?>>> _inflight = new();

    public HashtreeAttachmentCache(string dataDir, long cacheLimitBytes = DefaultCacheLimitBytes)
    {
        _attachmentsRoot = Path.Combine(dataDir, "attachments");
        _downloadedDir = Path.Combine(_attachmentsRoot, "downloaded");
        _cacheLimitBytes = cacheLimitBytes;
        Directory.CreateDirectory(_downloadedDir);
        Directory.CreateDirectory(Path.Combine(_attachmentsRoot, "outgoing"));
    }

    public string OutgoingDir => Path.Combine(_attachmentsRoot, "outgoing");

    public Task<byte[]?> ResolvePictureAsync(string nhash)
    {
        var trimmed = nhash?.Trim();
        if (string.IsNullOrEmpty(trimmed)) return Task.FromResult<byte[]?>(null);
        return ResolveAsync($"picture-{Safe(trimmed!)}", trimmed!);
    }

    public Task<byte[]?> ResolveAttachmentAsync(MessageAttachmentSnapshot attachment)
    {
        var key = $"{Safe(attachment.nhash)}-{Safe(attachment.filename)}";
        return ResolveAsync(key, attachment.nhash);
    }

    public string GetCachedAttachmentPath(MessageAttachmentSnapshot attachment, byte[] data)
    {
        var key = $"{Safe(attachment.nhash)}-{Safe(attachment.filename)}";
        var path = Path.Combine(_downloadedDir, key);
        if (!File.Exists(path)) File.WriteAllBytes(path, data);
        else File.SetLastWriteTime(path, DateTime.Now);
        Prune(path);
        return path;
    }

    /// Stage an outgoing file by copying into our local outbox so the Rust core
    /// can read it from a stable path while the user moves on.
    public (string Path, string Filename) StageOutgoing(string sourcePath)
    {
        var filename = Path.GetFileName(sourcePath);
        if (string.IsNullOrWhiteSpace(filename)) filename = "attachment";
        var dest = Path.Combine(OutgoingDir, $"{Guid.NewGuid()}-{filename}");
        File.Copy(sourcePath, dest, overwrite: true);
        return (dest, filename);
    }

    private async Task<byte[]?> ResolveAsync(string cacheKey, string nhash)
    {
        var path = Path.Combine(_downloadedDir, cacheKey);

        if (File.Exists(path))
        {
            try
            {
                File.SetLastWriteTime(path, DateTime.Now);
                return await File.ReadAllBytesAsync(path).ConfigureAwait(false);
            }
            catch
            {
                // Cache read failed; fall through to refetch.
            }
        }

        // Coalesce concurrent requests for the same key.
        var task = _inflight.GetOrAdd(
            cacheKey,
            _ => new Lazy<Task<byte[]?>>(() => DownloadAndCacheAsync(cacheKey, nhash, path), LazyThreadSafetyMode.ExecutionAndPublication)
        ).Value;

        try
        {
            return await task.ConfigureAwait(false);
        }
        finally
        {
            _inflight.TryRemove(cacheKey, out _);
        }
    }

    private async Task<byte[]?> DownloadAndCacheAsync(string cacheKey, string nhash, string path)
    {
        var data = await Task.Run(() =>
        {
            try
            {
                var result = Native.DownloadHashtreeAttachment(nhash);
                if (string.IsNullOrEmpty(result.dataBase64)) return null;
                return Convert.FromBase64String(result.dataBase64);
            }
            catch
            {
                return null;
            }
        }).ConfigureAwait(false);

        if (data == null || data.Length == 0) return null;

        try
        {
            await File.WriteAllBytesAsync(path, data).ConfigureAwait(false);
            Prune(path);
        }
        catch
        {
            // Cache write is best-effort.
        }

        return data;
    }

    private void Prune(string protectedPath)
    {
        try
        {
            var files = new DirectoryInfo(_downloadedDir).GetFiles();
            var total = 0L;
            foreach (var f in files) total += f.Length;
            if (total <= _cacheLimitBytes) return;

            var protectedFull = Path.GetFullPath(protectedPath);
            foreach (var f in files.OrderBy(f => f.LastWriteTime))
            {
                if (Path.GetFullPath(f.FullName) == protectedFull) continue;
                try { f.Delete(); total -= f.Length; } catch { }
                if (total <= _cacheLimitBytes) break;
            }
        }
        catch
        {
            // Pruning is best-effort.
        }
    }

    private static string Safe(string value)
    {
        var invalid = Path.GetInvalidFileNameChars();
        var s = string.IsNullOrWhiteSpace(value) ? "attachment" : value.Trim();
        var chars = new char[s.Length];
        for (int i = 0; i < s.Length; i++)
        {
            var c = s[i];
            chars[i] = (Array.IndexOf(invalid, c) >= 0 || c == ':' || c == '\\' || c == '/') ? '-' : c;
        }
        return new string(chars);
    }
}
