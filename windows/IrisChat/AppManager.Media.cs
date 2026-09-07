using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    public void SetImageProxyEnabled(bool enabled) =>
        DispatchToRust(new AppAction.SetImageProxyEnabled(enabled));

    public void SetImageProxyFallbackEnabled(bool enabled) =>
        DispatchToRust(new AppAction.SetImageProxyFallbackEnabled(enabled));

    public void SetImageProxyUrl(string url) =>
        DispatchToRust(new AppAction.SetImageProxyUrl(url.Trim()));

    public void SetImageProxyKeyHex(string keyHex) =>
        DispatchToRust(new AppAction.SetImageProxyKeyHex(keyHex.Trim()));

    public void SetImageProxySaltHex(string saltHex) =>
        DispatchToRust(new AppAction.SetImageProxySaltHex(saltHex.Trim()));

    public void ResetImageProxySettings() =>
        DispatchToRust(new AppAction.ResetImageProxySettings());
}
