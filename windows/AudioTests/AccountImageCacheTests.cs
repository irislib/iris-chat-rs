using System;
using IrisChat;

internal static class AccountImageCacheTests
{
    public static void Run()
    {
        var cache = new AccountImageCache<string>();
        var old = cache.Generation;
        if (!cache.TryStore("image", "private pixels", old)) throw new Exception("Initial cache store");
        cache.Clear();
        if (cache.TryGetValue("image", out _) || cache.IsCurrent(old)) throw new Exception("Logout retained cached image");
        if (cache.TryStore("image", "late private pixels", old)) throw new Exception("Old async image repopulated cache");
        var current = cache.Generation;
        if (!cache.TryStore("image", "new pixels", current)) throw new Exception("New account image rejected");
        cache.TryStore("image", "old pixels", old);
        if (!cache.TryGetValue("image", out var actual) || actual != "new pixels") throw new Exception("Old image replaced new account image");
        Console.WriteLine("PASS: logout clears image cache and rejects stale completions");
    }
}
