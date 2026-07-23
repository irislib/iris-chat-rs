using System;
using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    public SearchResultSnapshot Search(string query, uint limit = 50)
    {
        try
        {
            return _ffi.Search(query, null, limit);
        }
        catch (Exception error)
        {
            LogFfiFailure("ffi.search", error, $"query_len={query.Length}");
            return new SearchResultSnapshot(
                query,
                null,
                Array.Empty<FollowedUserSearchResult>(),
                Array.Empty<ChatThreadSnapshot>(),
                Array.Empty<ChatThreadSnapshot>(),
                Array.Empty<MessageSearchHit>(),
                null
            );
        }
    }
}
