using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    public void StartRemoteSignerLogin() => DispatchToRust(new AppAction.StartRemoteSignerLogin());

    public void ConnectRemoteSigner(string connectionUri) =>
        DispatchToRust(new AppAction.ConnectRemoteSigner(connectionUri));
}
