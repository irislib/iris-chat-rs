using System;
using System.Linq;
using System.Reflection;
using System.Threading;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class HistoryTests
{
    public static void Run(Window window, AppManager manager)
    {
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext());
        var all = Enumerable.Range(0, 240).Select(i => new ChatMessageSnapshot(
            i.ToString(), "synthetic", ChatMessageKind.User, "Synthetic person", null, null,
            $"Message {i}: " + string.Join(" ", Enumerable.Repeat("varied text", i % 12)),
            Array.Empty<MessageAttachmentSnapshot>(), Array.Empty<MessageReactionSnapshot>(), Array.Empty<MessageReactor>(),
            false, 100, null, DeliveryState.Received, Array.Empty<MessageRecipientDeliverySnapshot>(),
            new MessageDeliveryTraceSnapshot(Array.Empty<string>(), Array.Empty<string>(), Array.Empty<string>(), Array.Empty<string>(), null), null,
            editHistory: Array.Empty<MessageEditSnapshot>())).ToArray();
        var chat = manager.CurrentChat! with { messages = all.Skip(160).ToArray() };
        MergeChecks(manager, all);
        WarmWindowChecks(manager.State, all);
        int loads = 0;
        var list = new ItemsControl(); var scroll = new ScrollViewer { Content = list, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        var timeline = new MessageTimeline(scroll, list, () => loads++);
        window.Content = scroll; window.Height = 430; window.Width = 480;
        timeline.Update(chat, author => author); Pump();
        Near(scroll.VerticalOffset, scroll.ScrollableHeight, "Initial latest page");
        scroll.ScrollToVerticalOffset(60); Pump();
        Check(loads > 0, "Near-top scroll requests an older page");
        var anchor = list.Items.OfType<MessageBubble>().First(row => row.TranslatePoint(new Point(), scroll).Y + row.ActualHeight > 0);
        double Position() => anchor.TranslatePoint(new Point(), scroll).Y;
        var before = Position();
        chat = chat with { messages = all.Skip(80).ToArray() }; timeline.Update(chat, a => a); Pump();
        Near(Position(), before, "Second page keeps anchor");
        chat = chat with { messages = all }; timeline.Update(chat, a => a); Pump();
        Near(Position(), before, "Third page keeps anchor beyond 80 rows");
        Check(list.Items.Contains(anchor) && list.Items.Count == 240, "Rows retained through both prepends");
        var arrival = all[^1] with { id = "incoming", body = "New arrival" };
        chat = chat with { messages = all.Append(arrival).ToArray() }; timeline.Update(chat, a => a); Pump();
        Near(Position(), before, "Incoming arrival does not move browsed history");
        var changed = chat.messages.ToArray(); changed[5] = changed[5] with { body = string.Join("\n", Enumerable.Repeat("Edited text", 7)) };
        chat = chat with { messages = changed }; timeline.Update(chat, a => a); Pump();
        Near(Position(), before, "Changed row height preserves anchor");
        timeline.FollowLatest(); Pump();
        chat = chat with { messages = changed.Append(arrival with { id = "latest" }).ToArray() }; timeline.Update(chat, a => a); Pump();
        Near(scroll.VerticalOffset, scroll.ScrollableHeight, "Own send and subsequent arrivals follow latest");
        timeline.Reset(); timeline.Update(chat with { messages = all.Skip(160).ToArray() }, a => a); Pump();
        Near(scroll.VerticalOffset, scroll.ScrollableHeight, "Reopen discards old scroll intent");
        ReadStoredHistory(manager);
        Console.WriteLine("PASS: WPF 240 mixed-height messages, prepend/arrival/edit anchoring, authoritative history, actual async SQLite pages");
    }

    private static void MergeChecks(AppManager manager, ChatMessageSnapshot[] all)
    {
        var state = manager.State;
        var previous = state with { currentChat = state.currentChat! with { messages = all } };
        foreach (var messages in new[] { all.Skip(161).ToArray(), all.Skip(160).SkipLast(1).ToArray(), Array.Empty<ChatMessageSnapshot>() })
        {
            var fresh = state with { currentChat = state.currentChat! with { messages = messages, draft = "", displayName = "Fresh title" } };
            var merged = ChatHistory.PreservePage(previous, fresh, all.Skip(160).Select(m => m.id).ToHashSet());
            var expected = all.Take(160).Concat(messages).ToArray();
            Check(merged.currentChat!.messages.Select(m => m.id).SequenceEqual(expected.Select(m => m.id)), "Deleted recent first/last/all stay deleted");
            Check(merged.currentChat.displayName == "Fresh title" && merged.currentChat.draft == "", "Fresh metadata remains authoritative");
        }
        var current = new[] { all[80] with { body = "Fresh edit", reactions = new[] { new MessageReactionSnapshot("👍", 1, true) } }, all[81] };
        var page = all.Take(81).ToArray(); page[0] = page[0] with { expiresAtSecs = 1 };
        var result = ChatHistory.Merge(page, current);
        Check(result.Length == 81 && result[79].body == "Fresh edit" && result[79].reactions.Length == 1, "Late page preserves overlap edits/reactions and removes expired history");
        Check(result.Select(m => m.id).Distinct().Count() == result.Length, "History IDs unique with equal timestamps");
        var flags = BindingFlags.Instance | BindingFlags.NonPublic;
        var stateField = typeof(AppManager).GetField("_state", flags)!;
        var readIds = typeof(AppManager).GetField("_historyReadIds", flags)!;
        var generation = (long)typeof(AppManager).GetField("_historyGeneration", flags)!.GetValue(manager)!;
        var freshChat = state.currentChat! with { messages = new[] { all[81] with { body = "Newest edit" } }, displayName = "Newest metadata" };
        var latePage = state.currentChat with { messages = all.Skip(79).Take(3).ToArray() };
        stateField.SetValue(manager, state with { currentChat = freshChat });
        readIds.SetValue(manager, all.Skip(80).Take(2).Select(m => m.id).ToHashSet());
        try {
            Check((bool)typeof(AppManager).GetMethod("CompleteHistoryPage", flags)!.Invoke(manager,
                new object[] { generation, freshChat.chatId, "82", latePage })!, "Late page adds unseen history");
            Check(manager.CurrentChat!.messages.Select(m => m.id).SequenceEqual(new[] { "79", "81" }), "Late page cannot revive a deleted overlap");
            Check(manager.CurrentChat.messages[1].body == "Newest edit" && manager.CurrentChat.displayName == "Newest metadata", "Late page retains live edit and metadata");
            stateField.SetValue(manager, state with { currentChat = freshChat });
            var removed = (System.Collections.Generic.HashSet<string>)typeof(AppManager).GetField("_historyRemovedIds", flags)!.GetValue(manager)!;
            removed.Add("79");
            readIds.SetValue(manager, all.Skip(80).Take(2).Select(m => m.id).ToHashSet());
            Check(!(bool)typeof(AppManager).GetMethod("CompleteHistoryPage", flags)!.Invoke(manager,
                new object[] { generation, freshChat.chatId, "82", latePage })!, "Late page cannot revive a raw-known deleted older row");
            removed.Clear();
        } finally { stateField.SetValue(manager, state); }

    }

    private static void WarmWindowChecks(AppState state, ChatMessageSnapshot[] all)
    {
        var raw = state with { currentChat = state.currentChat! with { messages = all } };
        var excluded = new System.Collections.Generic.HashSet<string>();
        var shown = ChatHistory.PreservePage(state, raw, null, null, excluded, out var recent);
        Check(shown.currentChat!.messages.Select(m => m.id).SequenceEqual(all.Skip(160).Select(m => m.id)), "Warm240 enters with newest80");
        var rawIds = all.Select(m => m.id).ToHashSet();
        var unchanged = ChatHistory.PreservePage(shown, raw, recent, rawIds, excluded, out recent);
        Check(unchanged.currentChat!.messages.Length == 80, "Repeated warm snapshot stays80");
        shown = shown with { currentChat = shown.currentChat with { messages = all.Skip(80).ToArray() } };
        var changed = all.Where(m => m.id != "81").Select(m => m.id == "80"
            ? m with { body = "Fresh older edit", reactions = new[] { new MessageReactionSnapshot("👍", 1, false) } } : m)
            .Append(all[^1] with { id = "240", body = "Live arrival" }).ToArray();
        var nextRaw = raw with { currentChat = raw.currentChat! with { messages = changed, displayName = "Fresh metadata" } };
        shown = ChatHistory.PreservePage(shown, nextRaw, recent, rawIds, excluded, out recent);
        Check(shown.currentChat!.messages.Length == 160 && shown.currentChat.messages[0].id == "80"
            && shown.currentChat.messages[0].body == "Fresh older edit" && shown.currentChat.messages[0].reactions.Length == 1
            && shown.currentChat.messages.All(m => m.id != "81") && shown.currentChat.messages[^1].id == "240",
            "Loaded older rows receive edits/reactions/deletes; unseen prefix stays hidden and new arrival grows window");
        rawIds = changed.Select(m => m.id).ToHashSet();
        nextRaw = nextRaw with { currentChat = nextRaw.currentChat! with { messages = changed.Where(m => int.Parse(m.id) < 160).ToArray() } };
        shown = ChatHistory.PreservePage(shown, nextRaw, recent, rawIds, excluded, out recent);
        Check(shown.currentChat!.messages.Length == 79 && recent!.Count == 0, "Deleting all recent rows preserves only explicitly loaded older rows");
        var empty = nextRaw with { currentChat = nextRaw.currentChat! with { messages = Array.Empty<ChatMessageSnapshot>() } };
        shown = ChatHistory.PreservePage(shown, empty, recent, nextRaw.currentChat!.messages.Select(m => m.id).ToHashSet(), excluded, out _);
        Check(shown.currentChat!.messages.Length == 0, "Authoritative deletion reaches known older pages");
        Console.WriteLine("PASS: bounded warm240 window, raw older edits/reactions/deletes, live growth and same-timestamp ordering");
    }

    private static void ReadStoredHistory(AppManager manager)
    {
        var own = manager.Account!.publicKeyHex;
        manager.CreateChat(own); Until(() => manager.CurrentChat?.chatId == own);
        for (int i = 0; i < 240; i++) manager.SendMessage(own, $"History fixture {i:D3}");
        Until(() => manager.CurrentChat!.messages.Count(m => m.body.StartsWith("History fixture ")) == 240, 60);
        manager.NavigateBack(); manager.OpenChat(own);
        var ffi = (FfiApp)typeof(AppManager).GetField("_ffi", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(manager)!;
        // This existing FIFO query is a test-only barrier after OpenChat.
        // Resident core history remains 240; the shell must keep its entry at80.
        _ = ffi.ExportSupportBundleJson();
        var reopened = ffi.State();
        Check(reopened.currentChat?.messages.Length == 240, "Fixture retains a real warm240 core window");
        Until(() => manager.State.rev >= reopened.rev && manager.CurrentChat?.messages.Length == 80);
        var before = manager.CurrentChat!.messages[0].id;
        var page = manager.LoadOlderMessagesAsync(own); Until(() => page.IsCompleted);
        CheckPage(page.GetAwaiter().GetResult(), 160, before, "First real older-page query");
        before = manager.CurrentChat!.messages[0].id;
        page = manager.LoadOlderMessagesAsync(own); Until(() => page.IsCompleted);
        CheckPage(page.GetAwaiter().GetResult(), 240, before, "Second real older-page query");
        page = manager.LoadOlderMessagesAsync(own); Until(() => page.IsCompleted);
        Check(!page.GetAwaiter().GetResult(), "History stops at its real beginning");

        var generation = (long)typeof(AppManager).GetField("_historyGeneration", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(manager)!;
        var old = manager.CurrentChat!;
        manager.NavigateBack(); manager.OpenChat(own);
        var completion = typeof(AppManager).GetMethod("CompleteHistoryPage", BindingFlags.Instance | BindingFlags.NonPublic)!;
        Check(!(bool)completion.Invoke(manager, new object[] { generation, own, old.messages[0].id, old })!, "Route round-trip rejects a stale completion");

        void CheckPage(bool added, int expected, string cursor, string context)
        {
            var count = manager.CurrentChat?.messages.Length;
            if (added && count == expected) return;
            var persisted = ffi.ChatSnapshotBefore(own, cursor, 80);
            throw new Exception($"{context}: added={added}, displayed={count}, expected={expected}, "
                + $"persistedPage={persisted?.messages.Length}, cursor={cursor}, "
                + $"coreRows={ffi.State().currentChat?.messages.Length}, appliedRev={manager.State.rev}");
        }
    }
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); }
    private static void Near(double actual, double expected, string context) => Check(Math.Abs(actual - expected) <= 2, $"{context}: {actual} vs {expected}");
    private static void Pump()
    {
        var until = DateTime.UtcNow.AddMilliseconds(100);
        do { var frame = new DispatcherFrame(); Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle,
            new Action(() => frame.Continue = false)); Dispatcher.PushFrame(frame); Thread.Sleep(2); } while (DateTime.UtcNow < until);
    }
    private static void Until(Func<bool> condition, int seconds = 15)
    {
        var until = DateTime.UtcNow.AddSeconds(seconds);
        while (!condition()) { Check(DateTime.UtcNow < until, "History fixture timed out"); Pump(); }
    }
}
