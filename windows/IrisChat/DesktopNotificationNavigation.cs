using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;

namespace IrisChat;

public sealed record DesktopNotificationTarget(string Owner, string Chat, string Session);

/// A persisted session distinguishes a normal restart from logging out and
/// restoring the same account. Old Action Center entries cannot cross logout.
public sealed class DesktopNotificationNavigation
{
    public const string LaunchArgument = "--notification-target";
    private readonly string _sessionPath;
    private string _session;
    private DesktopNotificationTarget? _pending;
    private DateTimeOffset _expires;

    public DesktopNotificationNavigation(string dataDir)
    {
        _sessionPath = Path.Combine(dataDir, "notification-session");
        try { _session = File.ReadAllText(_sessionPath).Trim(); }
        catch { _session = ""; }
        if (!Guid.TryParseExact(_session, "N", out _)) RotateSession();
    }

    public DesktopNotificationTarget Target(string owner, string chat) => new(owner, chat, _session);
    public static string Encode(DesktopNotificationTarget target) => JsonSerializer.Serialize(target);
    public static DesktopNotificationTarget? Decode(string? payload)
    {
        if (string.IsNullOrEmpty(payload) || payload.Length > 2048) return null;
        try
        {
            var target = JsonSerializer.Deserialize<DesktopNotificationTarget>(payload);
            return target?.Owner is { Length: 64 } && target.Owner.All(Uri.IsHexDigit) &&
                !string.IsNullOrWhiteSpace(target.Chat) && target.Chat.Length <= 512 &&
                !target.Chat.Any(char.IsControl) && Guid.TryParseExact(target.Session, "N", out _) ? target : null;
        }
        catch (JsonException) { return null; }
    }

    public bool Accept(string payload)
    {
        var target = Decode(payload);
        if (target is null || target.Session != _session) return false;
        _pending = target;
        _expires = DateTimeOffset.UtcNow.AddMinutes(10);
        return true;
    }

    public string? TakeForAccount(string? owner, IEnumerable<string> knownChats)
    {
        var pending = _pending;
        if (pending is null) return null;
        if (DateTimeOffset.UtcNow > _expires) { ClearPending(); return null; }
        // Startup restoration or sign-in has not produced an account yet.
        if (owner is null) return null;
        if (!string.Equals(owner, pending.Owner, StringComparison.OrdinalIgnoreCase))
        { ClearPending(); return null; }
        // An early restored account may arrive before its chat list.
        if (!knownChats.Contains(pending.Chat, StringComparer.Ordinal)) return null;
        ClearPending();
        return pending.Chat;
    }

    public void ClearPending() => _pending = null;
    public void Invalidate()
    {
        ClearPending();
        RotateSession();
    }
    private void RotateSession()
    {
        _session = Guid.NewGuid().ToString("N");
        try { Directory.CreateDirectory(Path.GetDirectoryName(_sessionPath)!); File.WriteAllText(_sessionPath, _session); }
        catch { /* Warm activation remains usable if the data directory is unavailable. */ }
    }
}
