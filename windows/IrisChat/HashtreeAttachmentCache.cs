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
    private readonly ConcurrentDictionary<(long Generation, string Key), Lazy<Task<byte[]?>>> _inflight = new();
    private readonly object _gate = new();
    private readonly Func<string, Task<byte[]?>> _download;
    private long _generation;

    public long Generation { get { lock (_gate) return _generation; } }
    public bool IsCurrent(long generation) { lock (_gate) return generation == _generation; }

    // Must run before the account directory is deleted. Waiting on the same
    // short filesystem lock prevents a completed old download recreating it.
    public void Invalidate()
    {
        lock (_gate) { _generation++; _inflight.Clear(); }
    }

    public HashtreeAttachmentCache(string dataDir, long cacheLimitBytes = DefaultCacheLimitBytes, Func<string, Task<byte[]?>>? download = null)
    {
        _attachmentsRoot = Path.Combine(dataDir, "attachments");
        _downloadedDir = Path.Combine(_attachmentsRoot, "downloaded");
        _cacheLimitBytes = cacheLimitBytes;
        _download = download ?? DownloadAsync;
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

    public string? GetCachedAttachmentPath(MessageAttachmentSnapshot attachment, byte[] data, long generation)
    {
        lock (_gate)
        {
            if (generation != _generation) return null;
            Directory.CreateDirectory(_downloadedDir);
            var key = $"{Safe(attachment.nhash)}-{Safe(attachment.filename)}";
            var path = Path.Combine(_downloadedDir, key);
            if (!File.Exists(path)) File.WriteAllBytes(path, data);
            else File.SetLastWriteTime(path, DateTime.Now);
            Prune(path);
            return path;
        }
    }

    /// Stage an outgoing file by copying into our local outbox so the Rust core
    /// can read it from a stable path while the user moves on.
    public (string Path, string Filename) StageOutgoing(string sourcePath)
    {
        var filename = Path.GetFileName(sourcePath);
        if (string.IsNullOrWhiteSpace(filename)) filename = "attachment";
        lock (_gate)
        {
            Directory.CreateDirectory(OutgoingDir);
            var dest = Path.Combine(OutgoingDir, $"{Guid.NewGuid()}-{filename}");
            File.Copy(sourcePath, dest, overwrite: true);
            return (dest, filename);
        }
    }

    private async Task<byte[]?> ResolveAsync(string cacheKey, string nhash)
    {
        var generation = Generation;
        var path = Path.Combine(_downloadedDir, cacheKey);
        var cached = await Task.Run(() =>
        {
            lock (_gate)
            {
                if (generation != _generation || !File.Exists(path)) return null;
                try { File.SetLastWriteTime(path, DateTime.Now); return File.ReadAllBytes(path); }
                catch { return null; }
            }
        }).ConfigureAwait(false);
        if (!IsCurrent(generation)) return null;
        if (cached != null) return cached;

        // Old completion/removal must not disturb a new account's same-key load.
        var key = (generation, cacheKey);
        var task = _inflight.GetOrAdd(key,
            _ => new Lazy<Task<byte[]?>>(() => DownloadAndCacheAsync(nhash, path, generation), LazyThreadSafetyMode.ExecutionAndPublication)).Value;
        try { return await task.ConfigureAwait(false); }
        finally { _inflight.TryRemove(key, out _); }
    }

    private async Task<byte[]?> DownloadAndCacheAsync(string nhash, string path, long generation)
    {
        var data = await _download(nhash).ConfigureAwait(false);
        if (data == null || data.Length == 0) return null;
        lock (_gate)
        {
            if (generation != _generation) return null;
            try
            {
                Directory.CreateDirectory(_downloadedDir);
                File.WriteAllBytes(path, data);
                Prune(path);
            }
            catch { /* Cache write is best-effort. */ }
            return data;
        }
    }

    private static Task<byte[]?> DownloadAsync(string nhash) => Task.Run(() =>
    {
        try
        {
            var result = Native.DownloadHashtreeAttachment(nhash);
            return string.IsNullOrEmpty(result.dataBase64) ? null : Convert.FromBase64String(result.dataBase64);
        }
        catch { return null; }
    });

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
