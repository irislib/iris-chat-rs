using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    public void SetPublicFollow(string owner, bool following) => DispatchToRust(new AppAction.SetPublicFollow(owner, following));
    public void SetContactFavorite(string owner, bool favorite) => DispatchToRust(new AppAction.SetContactFavorite(owner, favorite));
    public void ApproveContactName(string owner, string name) => DispatchToRust(new AppAction.ApproveContactName(owner, name));
}
