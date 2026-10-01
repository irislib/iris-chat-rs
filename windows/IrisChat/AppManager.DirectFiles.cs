using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    public void SendDirectFiles(string chatId, IList<string> paths, string caption)
    {
        if (string.IsNullOrWhiteSpace(chatId) || paths.Count == 0) return;
        // The core owns the direct offer and reads these local files only after acceptance.
        // This path deliberately does not stage or upload through the attachment cache.
        try
        {
            var files = paths.Select(path => new OutgoingAttachment(path, Path.GetFileName(path))).ToArray();
            DispatchToRust(new AppAction.SendDirectFiles(chatId.Trim(), files, caption.Trim()));
        }
        catch { ShowToast("File could not be opened"); }
    }

    public void AcceptDirectFiles(string chatId, string transferId) =>
        DispatchToRust(new AppAction.AcceptDirectFiles(chatId, transferId));
    public void DeclineDirectFiles(string chatId, string transferId) =>
        DispatchToRust(new AppAction.DeclineDirectFiles(chatId, transferId));
    public void CancelDirectFiles(string chatId, string transferId) =>
        DispatchToRust(new AppAction.CancelDirectFiles(chatId, transferId));
}
