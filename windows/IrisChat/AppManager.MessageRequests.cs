using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    public void SetAcceptSocialGraphDirectMessages(bool enabled) =>
        DispatchToRust(new AppAction.SetAcceptSocialGraphDirectMessages(enabled));

    public void SetAcceptOvermutedDirectMessages(bool enabled) =>
        DispatchToRust(new AppAction.SetAcceptOvermutedDirectMessages(enabled));

    public void SetAcceptUnknownDirectMessages(bool enabled) =>
        DispatchToRust(new AppAction.SetAcceptUnknownDirectMessages(enabled));
}
