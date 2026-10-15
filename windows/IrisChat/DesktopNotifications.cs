using System;
using System.Collections.Generic;
using System.Linq;
using Microsoft.Toolkit.Uwp.Notifications;

namespace IrisChat;

/// Windows keeps these toasts in Action Center and can launch the unpackaged
/// app through the toolkit's COM activation registration after it has exited.
public sealed class SystemDesktopNotificationPoster : IDesktopNotificationPoster
{
    public static bool WasToastActivated => ToastNotificationManagerCompat.WasCurrentProcessToastActivated();
    public static void Register(Action<string> activated)
    {
        ToastNotificationManagerCompat.OnActivated += args =>
        {
            try
            {
                var values = ToastArguments.Parse(args.Argument);
                if (values.TryGetValue("target", out var payload)) activated(payload);
            }
            catch { /* Ignore malformed external activation arguments. */ }
        };
    }

    public void Post(string title, string body, DesktopNotificationTarget target)
    {
        try
        {
            new ToastContentBuilder()
                .AddArgument("target", DesktopNotificationNavigation.Encode(target))
                .AddText(title)
                .AddText(body)
                .Show(toast => {
                    toast.Tag = Guid.NewGuid().ToString("N")[..16];
                    toast.Group = target.Kind == "message" ? "iris-messages" : "iris-calls";
                });
        }
        catch { /* Notifications are best-effort; never interrupt message ingest. */ }
    }

    public void ClearRead(string dataDir, string owner, IReadOnlySet<string> fullyReadChats)
    {
        try
        {
            var records = new List<DesktopReadNotification>();
            foreach (var toast in ToastNotificationManagerCompat.History.GetHistory())
            {
                try
                {
                    var args = ToastArguments.Parse(toast.Content.DocumentElement.GetAttribute("launch"));
                    if (!args.TryGetValue("target", out var payload)) continue;
                    var target = DesktopNotificationNavigation.Decode(payload);
                    if (target != null) records.Add(new(toast.Tag, toast.Group, target));
                }
                catch { /* Old or unrelated toast metadata is not read proof. */ }
            }
            foreach (var item in DesktopReadNotification.Read(records, dataDir, owner, fullyReadChats))
                ToastNotificationManagerCompat.History.Remove(item.Tag, item.Group);
        }
        catch { /* Preserve notifications when OS history or storage is unavailable. */ }
    }

    public void Clear()
    {
        try { ToastNotificationManagerCompat.History.Clear(); } catch { }
    }
}
