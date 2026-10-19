using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    // Direct memory read in Rust, independent of the actor/database queues.
    internal UserProfileSnapshot UserProfile(string owner) => _readUserProfile(owner);
    public ulong ProfileRevision => _state.userDiscoveryRevision;

    public void SetPublicFollow(string owner, bool following) => DispatchToRust(new AppAction.SetPublicFollow(owner, following));
    public void SetContactFavorite(string owner, bool favorite) => DispatchToRust(new AppAction.SetContactFavorite(owner, favorite));
    public void ApproveContactName(string owner, string name) => DispatchToRust(new AppAction.ApproveContactName(owner, name));
}
