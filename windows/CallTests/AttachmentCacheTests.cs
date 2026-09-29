using System;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using IrisChat;

internal static class AttachmentCacheTests
{
    public static void Run() => VerifyAsync().GetAwaiter().GetResult();

    private static async Task VerifyAsync()
    {
        var directory = Path.Combine(Path.GetTempPath(), "iris-cache-logout-" + Guid.NewGuid());
        var oldBytes = new TaskCompletionSource<byte[]?>(TaskCreationOptions.RunContinuationsAsynchronously);
        var newBytes = new TaskCompletionSource<byte[]?>(TaskCreationOptions.RunContinuationsAsynchronously);
        var oldStarted = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var newStarted = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var calls = 0;
        var cache = new HashtreeAttachmentCache(directory, download: _ => {
            if (Interlocked.Increment(ref calls) == 1) { oldStarted.SetResult(); return oldBytes.Task; }
            newStarted.SetResult(); return newBytes.Task;
        });
        try
        {
            var oldLoad = cache.ResolvePictureAsync("same-image");
            await oldStarted.Task.WaitAsync(TimeSpan.FromSeconds(5)).ConfigureAwait(false);
            cache.Invalidate();
            Directory.Delete(directory, true);
            Directory.CreateDirectory(directory);
            var newLoad = cache.ResolvePictureAsync("same-image");
            await newStarted.Task.WaitAsync(TimeSpan.FromSeconds(5)).ConfigureAwait(false);
            oldBytes.SetResult(new byte[] { 1, 2, 3 });
            if (await oldLoad.ConfigureAwait(false) != null) throw new Exception("Logout delivered old attachment");
            if (Directory.GetFiles(directory, "*", SearchOption.AllDirectories).Length != 0) throw new Exception("Old download recreated deleted files");
            newBytes.SetResult(new byte[] { 4, 5, 6 });
            var current = await newLoad.ConfigureAwait(false);
            if (current == null || !current.SequenceEqual(new byte[] { 4, 5, 6 })) throw new Exception("New account download lost");
            var cached = await cache.ResolvePictureAsync("same-image").ConfigureAwait(false);
            if (calls != 2 || cached == null || !cached.SequenceEqual(current)) throw new Exception("Current cache must still work");
            Console.WriteLine("PASS: logout discards in-flight attachment writes while preserving new-session downloads");
        }
        finally { oldBytes.TrySetResult(null); newBytes.TrySetResult(null); if (Directory.Exists(directory)) Directory.Delete(directory, true); }
    }
}
