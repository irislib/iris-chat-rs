using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.Win32;

namespace IrisChat;

public sealed class UpdateService
{
    private const string RegistryPath = @"Software\Iris Chat";
    private static readonly Uri DefaultManifestUri = new(
        "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-chat-rs/latest/release.json"
    );
    private static readonly HttpClient Http = new();
    private static readonly JsonSerializerOptions JsonOptions = new() { PropertyNameCaseInsensitive = true };

    public static bool SkipOpen => Environment.GetEnvironmentVariable("IRIS_UPDATE_SKIP_OPEN") == "1";

    public async Task<UpdateResult> CheckAsync(string currentVersion)
    {
        var manifestUri = ManifestUri();
        var json = await ReadStringAsync(manifestUri).ConfigureAwait(false);
        var manifest = JsonSerializer.Deserialize<ReleaseManifest>(json, JsonOptions)
            ?? throw new InvalidOperationException("Release manifest was empty.");
        var asset = PreferredWindowsAsset(manifest.Assets);
        var available = VersionIsNewer(manifest.Tag, currentVersion);
        return new UpdateResult(
            available,
            manifest.Tag,
            asset?.Path is null ? null : new Uri(manifestUri, asset.Path),
            asset?.Name,
            available
                ? asset is null ? $"Update {manifest.Tag} found without a Windows app" : $"Update {manifest.Tag} available"
                : "Up to date"
        );
    }

    public async Task<string> DownloadAsync(Uri assetUri)
    {
        var downloadDir = Environment.GetEnvironmentVariable("IRIS_UPDATE_DOWNLOAD_DIR");
        if (string.IsNullOrWhiteSpace(downloadDir))
        {
            downloadDir = Path.Combine(Path.GetTempPath(), "IrisChatDownloads");
        }
        Directory.CreateDirectory(downloadDir);

        var fileName = Path.GetFileName(assetUri.LocalPath);
        if (string.IsNullOrWhiteSpace(fileName))
        {
            fileName = "iris-chat-update.exe";
        }
        var destination = Path.Combine(downloadDir, fileName);
        if (File.Exists(destination))
        {
            File.Delete(destination);
        }

        if (assetUri.IsFile)
        {
            File.Copy(assetUri.LocalPath, destination);
        }
        else
        {
            await using var stream = await Http.GetStreamAsync(assetUri).ConfigureAwait(false);
            await using var file = File.Create(destination);
            await stream.CopyToAsync(file).ConfigureAwait(false);
        }
        return destination;
    }

    public static bool LoadAutoCheckUpdates() =>
        RegistryDword("AutoCheckUpdates", defaultValue: 1) != 0;

    public static bool LoadAutoInstallUpdates() =>
        RegistryDword("AutoInstallUpdates", defaultValue: 0) != 0;

    public static void SaveAutoCheckUpdates(bool enabled) =>
        SaveRegistryDword("AutoCheckUpdates", enabled);

    public static void SaveAutoInstallUpdates(bool enabled) =>
        SaveRegistryDword("AutoInstallUpdates", enabled);

    private static Uri ManifestUri()
    {
        var overrideUrl = Environment.GetEnvironmentVariable("IRIS_UPDATE_MANIFEST_URL");
        return string.IsNullOrWhiteSpace(overrideUrl) ? DefaultManifestUri : new Uri(overrideUrl);
    }

    private static Task<string> ReadStringAsync(Uri uri) =>
        uri.IsFile ? File.ReadAllTextAsync(uri.LocalPath) : Http.GetStringAsync(uri);

    private static ReleaseAsset? PreferredWindowsAsset(IEnumerable<ReleaseAsset> assets) =>
        assets.FirstOrDefault(asset => asset.Name.EndsWith("windows-x64-setup.exe", StringComparison.OrdinalIgnoreCase));

    private static bool VersionIsNewer(string candidate, string current)
    {
        // Dev builds inherit AssemblyVersion=1.0.0.0 (or other pre-year-style
        // placeholders) because release builds set Version via dotnet publish.
        // Releases use "YYYY.M.D[.N]" so anything with a major below 2000 is a
        // local build that should never be told a release is newer.
        if (IsDevPlaceholderVersion(current))
        {
            return false;
        }
        var left = VersionParts(candidate);
        var right = VersionParts(current);
        var count = Math.Max(left.Count, right.Count);
        for (var i = 0; i < count; i++)
        {
            var leftValue = i < left.Count ? left[i] : 0;
            var rightValue = i < right.Count ? right[i] : 0;
            if (leftValue != rightValue)
            {
                return leftValue > rightValue;
            }
        }
        return false;
    }

    private static bool IsDevPlaceholderVersion(string current)
    {
        var parts = VersionParts(current);
        return parts.Count == 0 || parts[0] < 2000;
    }

    private static List<int> VersionParts(string value) =>
        value
            .Trim()
            .TrimStart('v', 'V')
            .Split('.', '-', '+')
            .Select(part => int.TryParse(new string(part.TakeWhile(char.IsDigit).ToArray()), out var parsed) ? parsed : 0)
            .ToList();

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

public sealed record UpdateResult(bool Available, string Tag, Uri? AssetUrl, string? AssetName, string Message);

public sealed class ReleaseManifest
{
    public string Tag { get; set; } = "";
    public List<ReleaseAsset> Assets { get; set; } = [];
}

public sealed class ReleaseAsset
{
    public string Name { get; set; } = "";
    public string Path { get; set; } = "";
}
