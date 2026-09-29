using System;
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
                .Show();
        }
        catch { /* Notifications are best-effort; never interrupt message ingest. */ }
    }

    public void Clear()
    {
        try { ToastNotificationManagerCompat.History.Clear(); } catch { }
    }
}
