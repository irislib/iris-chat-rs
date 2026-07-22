using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    private void TryRestorePersistedSession()
    {
        var pendingLink = _pendingDeviceLinkSecretStore.LoadPendingDeviceLink();
        if (pendingLink != null)
        {
            _persistedRestoreInFlight = true;
            BootstrapInFlight = true;
            Notify(nameof(BootstrapInFlight));
            var restored = DispatchToRust(new AppAction.RestorePendingDeviceLink(
                pendingLink.DeviceNsec,
                pendingLink.ApprovalBootstrapJson
            ), showToastOnFailure: false);
            if (!restored)
            {
                _persistedRestoreInFlight = false;
                BootstrapInFlight = false;
                Notify(nameof(BootstrapInFlight));
            }
            return;
        }

        var bundle = _secretStore.Load();
        if (bundle == null)
        {
            BootstrapInFlight = false;
            Notify(nameof(BootstrapInFlight));
            return;
        }
        _persistedRestoreInFlight = true;
        BootstrapInFlight = true;
        Notify(nameof(BootstrapInFlight));
        var dispatched = DispatchToRust(new AppAction.RestoreAccountBundle(
            bundle.OwnerNsec,
            bundle.OwnerPubkeyHex,
            bundle.DeviceNsec
        ), showToastOnFailure: false);
        if (!dispatched)
        {
            _persistedRestoreInFlight = false;
            BootstrapInFlight = false;
            Notify(nameof(BootstrapInFlight));
        }
    }
}
