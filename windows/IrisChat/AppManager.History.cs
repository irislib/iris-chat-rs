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
    private HashSet<string> _historyReadIds = new();

    private void ResetHistoryPaging()
    {
        _historyGeneration++;
        _historyScope = _historyLoadingBefore = _historyExhaustedBefore = null;
        _historyRecentIds = null;
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
        var scope = HistoryScope(incoming);
        var recentIds = scope != null && scope == _historyScope ? _historyRecentIds : null;
        if (scope != _historyScope) ResetHistoryPaging();
        _historyScope = scope;
        _historyRecentIds = incoming.currentChat?.messages.Select(message => message.id).ToHashSet();
        return ChatHistory.PreservePage(previous, incoming, recentIds);
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
        var validPage = page.messages.Where(message => !_historyReadIds.Contains(message.id) || currentIds.Contains(message.id)).ToArray();
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

    public static AppState PreservePage(AppState previous, AppState incoming, HashSet<string>? oldRecentIds)
    {
        if (previous.account?.publicKeyHex != incoming.account?.publicKeyHex
            || previous.currentChat is not {} oldChat || incoming.currentChat is not {} nextChat
            || oldChat.chatId != nextChat.chatId) return incoming;
        if (oldRecentIds == null) return incoming;
        return incoming with { currentChat = nextChat with {
            messages = Merge(oldChat.messages.Where(message => !oldRecentIds.Contains(message.id)).ToArray(), nextChat.messages) } };
    }
}
