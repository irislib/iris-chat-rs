using IrisChat.Bindings;

namespace IrisChat;

public sealed partial class AppManager
{
    public void SetChatDraft(string chatId, string text) =>
        DispatchToRust(new AppAction.SetChatDraft(chatId, text));

    public void EditMessage(string chatId, string messageId, string text) =>
        DispatchToRust(new AppAction.EditMessage(chatId, messageId, text.Trim()));

    public void DeleteMessageForEveryone(string chatId, string messageId) =>
        DispatchToRust(new AppAction.DeleteMessageForEveryone(chatId, messageId));

    public void SetAllowMessageDeletionByOthers(bool enabled) =>
        DispatchToRust(new AppAction.SetAllowMessageDeletionByOthers(enabled));
}
