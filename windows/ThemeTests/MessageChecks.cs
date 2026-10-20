using System;
using System.Windows;
using System.Windows.Controls;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class MessageChecks
{
    internal static FrameworkElement Build(AppManager manager)
    {
        var messages = new StackPanel { Margin = new Thickness(20) };
        var time = (ulong)DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        var sample = new ChatMessageSnapshot("theme", manager.CurrentChat!.chatId, ChatMessageKind.User,
            "Alex", null, null, "Readable text and https://example.invalid/link", [], [], [], false, time, null,
            DeliveryState.Seen, [], new MessageDeliveryTraceSnapshot([], [], [], [], null), null);
        var audio = new MessageAttachmentSnapshot(uploadedUrls: [], nhash: "theme", filename: "voice.wav",
            filenameEncoded: "voice.wav", htreeUrl: "htree://theme/voice.wav", isImage: false, isVideo: false, isAudio: true);
        var file = audio with { filename = "notes.txt", isAudio = false };
        var transfer = new DirectFileTransferSnapshot(id: "theme-transfer",
            files: new[] { new DirectFileSnapshot(filename: "photo.jpg", sizeBytes: 1024, localPath: null) },
            status: DirectFileTransferStatus.Offered, isSender: false, transferredBytes: 0, totalBytes: 1024, error: null);
        foreach (var outgoing in new[] { false, true })
        {
            var text = new MessageBubble();
            text.Bind(sample with { isOutgoing = outgoing,
                editHistory = new[] { new MessageEditSnapshot("edit", sample.body, time) } },
                showAuthor: true, authorLabel: "Alex");
            messages.Children.Add(text);
            var attachments = new MessageBubble();
            attachments.Bind(sample with { id = "attachments-" + outgoing, isOutgoing = outgoing,
                body = "Audio and files", attachments = new[] { audio, file },
                directTransfer = transfer with { isSender = outgoing } });
            messages.Children.Add(attachments);
        }
        var system = new MessageBubble();
        system.Bind(sample with { kind = ChatMessageKind.System, body = "Messages stay private" });
        messages.Children.Add(system);
        var avatars = new WrapPanel();
        for (var i = 0; i < 40; i++)
            avatars.Children.Add(new Avatar { Label = "Color " + i, Margin = new Thickness(4),
                SocialConnection = new SocialConnectionSnapshot(
                    i % 4 == 0 ? SocialBadge.Trusted : i % 4 == 1 ? SocialBadge.Following
                    : i % 4 == 2 ? SocialBadge.Muted : SocialBadge.Warning, 1, 0, "Following", true) });
        messages.Children.Add(avatars);
        messages.Children.Add(new ComposerBar());
        return new ScrollViewer { Content = messages, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    }
}
