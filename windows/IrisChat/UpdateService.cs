using System;
using Microsoft.Win32;

namespace IrisChat;

/// Update preferences. The Rust core runs the update itself.
public sealed class UpdateService
{
    private const string RegistryPath = @"Software\Iris Chat";

    public static bool SkipOpen => Environment.GetEnvironmentVariable("IRIS_UPDATE_SKIP_OPEN") == "1";

    public static bool LoadAutoCheckUpdates() =>
        RegistryDword("AutoCheckUpdates", defaultValue: 1) != 0;

    public static bool LoadAutoInstallUpdates() =>
        RegistryDword("AutoInstallUpdates", defaultValue: 0) != 0;

    public static void SaveAutoCheckUpdates(bool enabled) =>
        SaveRegistryDword("AutoCheckUpdates", enabled);

    public static void SaveAutoInstallUpdates(bool enabled) =>
        SaveRegistryDword("AutoInstallUpdates", enabled);

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
