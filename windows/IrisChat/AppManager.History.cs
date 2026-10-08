using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    private long _historyGeneration;
    private string? _historyScope;
    private string? _historyLoadingBefore;
    private string? _historyExhaustedBefore;
    private HashSet<string>? _historyRecentIds;
    private HashSet<string>? _historyRawIds;
    private readonly HashSet<string> _historyExcludedIds = new();
    private readonly HashSet<string> _historyRemovedIds = new();
    private HashSet<string> _historyReadIds = new();

    private void ResetHistoryPaging()
    {
        _historyGeneration++;
        _historyScope = _historyLoadingBefore = _historyExhaustedBefore = null;
        _historyRecentIds = _historyRawIds = null;
        _historyExcludedIds.Clear();
        _historyRemovedIds.Clear();
        _historyReadIds.Clear();
    }

    private static string? HistoryScope(AppState state)
    {
        var active = state.router.screenStack.LastOrDefault() ?? state.router.defaultScreen;
        return state.account != null && active is Screen.Chat route && state.currentChat?.chatId == route.chatId
            ? state.account.publicKeyHex + ":" + route.chatId : null;
    }

    private void UpdateHistoryScope()
    {
        var scope = HistoryScope(_state);
        if (_historyScope == scope) return;
        ResetHistoryPaging();
        _historyScope = scope;
        _historyRecentIds = CurrentChat?.messages.Select(message => message.id).ToHashSet();
    }

    private AppState ReconcileHistory(AppState previous, AppState incoming)
    {
        if (previous.messageVisibilityRevision != incoming.messageVisibilityRevision) ResetHistoryPaging();
        var scope = HistoryScope(incoming);
        var recentIds = scope != null && scope == _historyScope ? _historyRecentIds : null;
        if (scope != _historyScope) ResetHistoryPaging();
        _historyScope = scope;
        var rawIds = incoming.currentChat?.messages.Select(message => message.id).ToHashSet();
        if (_historyRawIds != null && rawIds != null)
            _historyRemovedIds.UnionWith(_historyRawIds.Except(rawIds));
        if (rawIds != null) _historyRemovedIds.ExceptWith(rawIds);
        var next = ChatHistory.PreservePage(previous, incoming, recentIds,
            _historyRawIds, _historyExcludedIds, out var nextRecentIds);
        _historyRecentIds = nextRecentIds;
        _historyRawIds = rawIds;
        return next;
    }

    public async Task<bool> LoadOlderMessagesAsync(string chatId)
    {
        UpdateHistoryScope();
        var current = CurrentChat;
        if (_historyScope == null || current?.chatId != chatId || current.messages.Length == 0) return false;
        var before = current.messages[0].id;
        if (_historyLoadingBefore != null || _historyExhaustedBefore == before) return false;
        var generation = _historyGeneration;
        _historyLoadingBefore = before;
        _historyReadIds = current.messages.Select(message => message.id).ToHashSet();
        CurrentChatSnapshot? page = null;
        try { page = await Task.Run(() => _ffi.ChatSnapshotBefore(chatId, before, RouteChatSnapshotLimit)); }
        catch (Exception error) { LogFfiFailure("ffiapp.chat_snapshot_before", error); }
        return CompleteHistoryPage(generation, chatId, before, page);
    }

    private bool CompleteHistoryPage(long generation, string chatId, string before, CurrentChatSnapshot? page)
    {
        if (generation != _historyGeneration) return false;
        _historyLoadingBefore = null;
        var current = CurrentChat;
        if (page?.chatId != chatId || current?.chatId != chatId) return false;
        if (page.messages.Length < RouteChatSnapshotLimit) _historyExhaustedBefore = page.messages.FirstOrDefault()?.id ?? before;
        var currentIds = current.messages.Select(message => message.id).ToHashSet();
        var validPage = page.messages.Where(message => !_historyRemovedIds.Contains(message.id)
            && (!_historyReadIds.Contains(message.id) || currentIds.Contains(message.id))).ToArray();
        _historyReadIds.Clear();
        var messages = ChatHistory.Merge(validPage, current.messages);
        if (messages.Length == current.messages.Length) return false;
        // A page may complete after a new message, receipt or metadata update.
        _state = _state with { currentChat = current with { messages = messages } };
        NotifyAll();
        return true;
    }
}

internal static class ChatHistory
{
    // The second input is authoritative; stable sorting keeps Rust's order for
    // equal timestamps, including local messages waiting for their final ID.
    public static ChatMessageSnapshot[] Merge(ChatMessageSnapshot[] older, ChatMessageSnapshot[] current)
    {
        var ids = current.Select(message => message.id).ToHashSet();
        var now = (ulong)DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        var retained = older.Where(message => (message.expiresAtSecs == null || message.expiresAtSecs > now)
            && ids.Add(message.id)).ToArray();
        if (retained.Length == 0) return current;
        return retained.Concat(current).OrderBy(message => message.createdAtSecs).ToArray();
    }

    public static AppState PreservePage(AppState previous, AppState incoming, HashSet<string>? oldRecentIds) =>
        PreservePage(previous, incoming, oldRecentIds, null, new HashSet<string>(), out _);

    public static AppState PreservePage(AppState previous, AppState incoming, HashSet<string>? oldRecentIds,
        HashSet<string>? oldRawIds, HashSet<string> excludedIds, out HashSet<string>? recentIds)
    {
        recentIds = null;
        if (incoming.currentChat is not { } nextChat) return incoming;
        var raw = nextChat.messages;
        // A resident core may still hold thousands of rows. Opening a chat only
        // exposes its newest page; live arrivals may extend that window.
        var start = oldRecentIds == null ? -1 : Array.FindIndex(raw, m => oldRecentIds.Contains(m.id));
        if (start < 0 && oldRawIds == null) start = Math.Max(0, raw.Length - 80);
        if (start > 0) excludedIds.UnionWith(raw.Take(start).Select(m => m.id));
        var recent = raw.Where(m => !excludedIds.Contains(m.id)).ToArray();
        recentIds = recent.Select(m => m.id).ToHashSet();
        var messages = recent;
        if (previous.account?.publicKeyHex == incoming.account?.publicKeyHex
            && previous.currentChat is { } oldChat && oldChat.chatId == nextChat.chatId
            && oldRecentIds != null)
        {
            var fresh = raw.ToDictionary(m => m.id);
            var older = oldChat.messages.Where(m => !oldRecentIds.Contains(m.id))
                .Where(m => fresh.ContainsKey(m.id) || oldRawIds == null || !oldRawIds.Contains(m.id))
                .Select(m => fresh.GetValueOrDefault(m.id, m)).ToArray();
            messages = Merge(older, recent);
        }
        return incoming with { currentChat = nextChat with { messages = messages } };
    }
}
