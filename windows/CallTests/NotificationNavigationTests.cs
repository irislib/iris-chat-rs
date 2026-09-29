using System;
using System.IO;
using System.Reflection;
using System.Threading;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Bindings;

internal static class NotificationNavigationTests
{
    internal static void Run(AppManager manager, string output)
    {
        var directory = Path.Combine(output, "notification-routing");
        Directory.CreateDirectory(directory);
        var owner = new string('a', 64);
        var first = new string('b', 64);
        var second = "group:" + new string('c', 64);
        var router = new DesktopNotificationNavigation(directory);
        var firstPayload = DesktopNotificationNavigation.Encode(router.Target(owner, first));
        var secondPayload = DesktopNotificationNavigation.Encode(router.Target(owner, second));
        Check(router.Accept(firstPayload), "valid notification accepted");
        Check(router.TakeForAccount(null, Array.Empty<string>()) == null, "waits for session restoration");
        Check(router.TakeForAccount(owner, Array.Empty<string>()) == null, "waits for chat restoration");
        Check(router.TakeForAccount(owner, new[] { first, second }) == first, "opens original notification's chat");
        Check(router.TakeForAccount(owner, new[] { first }) == null, "one click navigates once");
        router.Accept(firstPayload); router.Accept(secondPayload);
        Check(router.TakeForAccount(owner, new[] { first, second }) == second, "latest deliberate click wins");
        var restarted = new DesktopNotificationNavigation(directory);
        Check(restarted.Accept(firstPayload) && restarted.TakeForAccount(owner, new[] { first }) == first,
            "cold-start activation survives a normal process restart");
        restarted.Accept(firstPayload);
        Check(restarted.TakeForAccount(new string('d', 64), new[] { first }) == null &&
              restarted.TakeForAccount(owner, new[] { first }) == null, "different account consumes stale activation");
        restarted.Accept(firstPayload); restarted.Invalidate();
        Check(!restarted.Accept(firstPayload) && !new DesktopNotificationNavigation(directory).Accept(firstPayload),
            "logout invalidates both queued and persisted old notifications");
        Check(!restarted.Accept("{}") && !restarted.Accept("not json") &&
              !restarted.Accept("{\"Owner\":null,\"Chat\":null,\"Session\":null}"), "malformed activation ignored");

        // Exercise the real AppManager method and its optimistic navigation,
        // backed by the actual Rust app, not a second routing implementation.
        manager.CreateAccount("Notification test");
        PumpUntil(() => manager.Account != null);
        const string peer = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        manager.CreateChat(peer);
        PumpUntil(() => Array.Exists(manager.State.chatList, chat => chat.chatId == peer));
        var liveRouter = (DesktopNotificationNavigation)typeof(AppManager)
            .GetField("_notificationNavigation", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(manager)!;
        var payload = DesktopNotificationNavigation.Encode(liveRouter.Target(manager.Account!.publicKeyHex, peer));
        manager.OpenNotification(payload);
        Check(Native.RouterOpenChatId(manager.State.router) == peer, "real AppManager opens matching notification chat");
        Console.WriteLine("PASS: Windows notification warm/cold routing, pending auth, exact target, stale-account/logout safeguards");
    }

    private static void PumpUntil(Func<bool> ready)
    {
        var end = DateTime.UtcNow.AddSeconds(10);
        while (!ready())
        {
            if (DateTime.UtcNow > end) throw new Exception("Notification routing fixture timed out");
            var frame = new DispatcherFrame();
            Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.Background, new Action(() => frame.Continue = false));
            Dispatcher.PushFrame(frame);
            Thread.Sleep(5);
        }
    }
    private static void Check(bool condition, string message)
    { if (!condition) throw new Exception(message); }
}
