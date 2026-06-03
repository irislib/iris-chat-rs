using System;
using System.IO;
using System.Threading.Tasks;
using IrisChat.Bindings;
using Microsoft.Win32;

namespace IrisChat;

public sealed class UpdateService
{
    private const string RegistryPath = @"Software\Iris Chat";

    public static bool SkipOpen => Environment.GetEnvironmentVariable("IRIS_UPDATE_SKIP_OPEN") == "1";

    public async Task<UpdateResult> CheckAsync()
    {
        var update = await Task.Run(Native.IrisDesktopUpdateCheck).ConfigureAwait(false);
        EnsureUpdateSucceeded(update);
        var hasAsset = !string.IsNullOrWhiteSpace(update.asset);
        return new UpdateResult(
            update.available,
            update.tag,
            hasAsset ? update.asset : null,
            update.available
                ? hasAsset ? $"Update {update.tag} available" : $"Update {update.tag} found without a Windows app"
                : "Up to date",
            HasAsset: hasAsset
        );
    }

    public async Task<string> DownloadWithEmbeddedUpdaterAsync()
    {
        var downloadDir = Environment.GetEnvironmentVariable("IRIS_UPDATE_DOWNLOAD_DIR");
        if (string.IsNullOrWhiteSpace(downloadDir))
        {
            downloadDir = Path.Combine(Path.GetTempPath(), "IrisChatDownloads");
        }
        Directory.CreateDirectory(downloadDir);
        var update = await Task.Run(() => Native.IrisDesktopUpdateDownload(downloadDir)).ConfigureAwait(false);
        EnsureUpdateSucceeded(update);
        if (string.IsNullOrWhiteSpace(update.path))
        {
            throw new InvalidOperationException("Downloaded update was not found.");
        }
        return update.path;
    }

    public static bool LoadAutoCheckUpdates() =>
        RegistryDword("AutoCheckUpdates", defaultValue: 1) != 0;

    public static bool LoadAutoInstallUpdates() =>
        RegistryDword("AutoInstallUpdates", defaultValue: 0) != 0;

    public static void SaveAutoCheckUpdates(bool enabled) =>
        SaveRegistryDword("AutoCheckUpdates", enabled);

    public static void SaveAutoInstallUpdates(bool enabled) =>
        SaveRegistryDword("AutoInstallUpdates", enabled);

    private static void EnsureUpdateSucceeded(IrisDesktopUpdateResult update)
    {
        if (!update.ok)
        {
            throw new InvalidOperationException(string.IsNullOrWhiteSpace(update.error)
                ? "Update failed."
                : update.error);
        }
        if (Environment.GetEnvironmentVariable("IRIS_UPDATE_MANIFEST_URL") is null
            && update.available
            && (!update.verified || !string.Equals(update.source, "hashtree-nostr-blossom", StringComparison.Ordinal)))
        {
            throw new InvalidOperationException("Update could not be verified.");
        }
    }

    private static int RegistryDword(string name, int defaultValue)
    {
        using var key = Registry.CurrentUser.OpenSubKey(RegistryPath);
        return key?.GetValue(name) is int value ? value : defaultValue;
    }

    private static void SaveRegistryDword(string name, bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(RegistryPath);
        key?.SetValue(name, enabled ? 1 : 0, RegistryValueKind.DWord);
    }
}

public sealed record UpdateResult(
    bool Available,
    string Tag,
    string? AssetName,
    string Message,
    bool HasAsset);
