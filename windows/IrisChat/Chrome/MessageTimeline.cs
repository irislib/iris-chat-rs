using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

// The viewport owns scroll intent independently of the latest message snapshot.
internal sealed class MessageTimeline
{
    private readonly ScrollViewer _scroll;
    private readonly ItemsControl _list;
    private readonly Action _loadOlder;
    private readonly Action<ChatMessageSnapshot>? _editMessage;
    private readonly Dictionary<string, Row> _rows = new();
    private bool _following = true;
    private bool _updating;
    private int _generation;
    private (string Id, double Y)? _anchor;
    private double _heldOffset;
    private sealed record Row(MessageBubble View, ChatMessageSnapshot Message, bool Author, string Label,
        MessageGrouping.Layout Grouping);

    public MessageTimeline(ScrollViewer scroll, ItemsControl list, Action loadOlder,
        Action<ChatMessageSnapshot>? editMessage = null)
    {
        _scroll = scroll; _list = list; _loadOlder = loadOlder;
        _editMessage = editMessage;
        scroll.ScrollChanged += (_, e) =>
        {
            if (_updating) return;
            if (e.ExtentHeightChange != 0) { SchedulePlacement(); return; }
            if (e.VerticalChange == 0) return;
            _following = scroll.VerticalOffset >= scroll.ScrollableHeight - 24;
            CaptureAnchor();
            if (scroll.VerticalOffset < 120 && scroll.ScrollableHeight > 0) _loadOlder();
        };
    }

    private static bool SameMessage(ChatMessageSnapshot left, ChatMessageSnapshot right)
    {
        // Generated records compare arrays by reference. Keep unchanged rows
        // even when a full core snapshot supplies freshly allocated arrays.
        var trace = left.deliveryTrace with {
            outerEventIds = right.deliveryTrace.outerEventIds,
            pendingRelayEventIds = right.deliveryTrace.pendingRelayEventIds,
            queuedProtocolTargets = right.deliveryTrace.queuedProtocolTargets,
            transportChannels = right.deliveryTrace.transportChannels };
        var transfer = left.directTransfer == null ? null : left.directTransfer with {
            files = right.directTransfer?.files ?? Array.Empty<DirectFileSnapshot>() };
        return left with { attachments = right.attachments, reactions = right.reactions, reactors = right.reactors,
            recipientDeliveries = right.recipientDeliveries, deliveryTrace = trace, directTransfer = transfer,
            editHistory = right.editHistory } == right
            && (left.editHistory ?? []).SequenceEqual(right.editHistory ?? [])
            && left.attachments.SequenceEqual(right.attachments)
            && left.reactions.SequenceEqual(right.reactions) && left.reactors.SequenceEqual(right.reactors)
            && left.recipientDeliveries.SequenceEqual(right.recipientDeliveries)
            && left.deliveryTrace.outerEventIds.SequenceEqual(right.deliveryTrace.outerEventIds)
            && left.deliveryTrace.pendingRelayEventIds.SequenceEqual(right.deliveryTrace.pendingRelayEventIds)
            && left.deliveryTrace.queuedProtocolTargets.SequenceEqual(right.deliveryTrace.queuedProtocolTargets)
            && left.deliveryTrace.transportChannels.SequenceEqual(right.deliveryTrace.transportChannels)
            && (left.directTransfer == null || right.directTransfer != null
                && left.directTransfer.files.SequenceEqual(right.directTransfer.files));
    }

    public void Reset()
    {
        _generation++; _updating = false; _following = true; _anchor = null;
        _rows.Clear(); _list.Items.Clear();
    }

    public void FollowLatest() { _following = true; _scroll.ScrollToBottom(); }

    public void Update(CurrentChatSnapshot chat, Func<string, string> authorLabel)
    {
        if (!_updating) CaptureAnchor();
        var desired = new List<MessageBubble>();
        var ids = new HashSet<string>();
        ChatMessageSnapshot? previous = null;
        var changed = false;
        for (var index = 0; index < chat.messages.Length; index++)
        {
            var message = chat.messages[index];
            if (!ids.Add(message.id)) continue;
            var grouping = MessageGrouping.For(previous, message, chat.messages.ElementAtOrDefault(index + 1), chat.kind);
            var author = chat.kind == ChatKind.Group && !message.isOutgoing && grouping.Start;
            var label = authorLabel(message.author);
            if (!_rows.TryGetValue(message.id, out var row))
            {
                row = new Row(new MessageBubble { Uid = message.id }, message, author, label, grouping);
                row.View.EditRequested += selected => _editMessage?.Invoke(selected);
                row.View.Bind(message, author, label, grouping.Start, grouping.End, grouping.Footer);
                _rows.Add(message.id, row); changed = true;
            }
            else if (!SameMessage(row.Message, message) || row.Author != author || row.Label != label || row.Grouping != grouping)
            {
                row.View.Bind(message, author, label, grouping.Start, grouping.End, grouping.Footer);
                _rows[message.id] = row with { Message = message, Author = author, Label = label, Grouping = grouping };
                changed = true;
            }
            desired.Add(row.View); previous = message;
        }
        if (!changed && ids.Count == _rows.Count && _list.Items.OfType<MessageBubble>().SequenceEqual(desired)) return;
        _updating = true;
        foreach (var id in _rows.Keys.Where(id => !ids.Contains(id)).ToArray())
        {
            _list.Items.Remove(_rows[id].View); _rows.Remove(id);
        }
        for (var i = 0; i < desired.Count; i++)
        {
            if (i < _list.Items.Count && ReferenceEquals(_list.Items[i], desired[i])) continue;
            _list.Items.Remove(desired[i]); _list.Items.Insert(i, desired[i]);
        }
        SchedulePlacement();
    }

    private void CaptureAnchor()
    {
        _heldOffset = _scroll.VerticalOffset;
        _anchor = null;
        if (_following) return;
        foreach (var row in _list.Items.OfType<MessageBubble>())
        {
            var y = row.TranslatePoint(new Point(), _scroll).Y;
            if (y + row.ActualHeight > 0) { _anchor = (row.Uid, y); break; }
        }
    }

    private void SchedulePlacement()
    {
        _updating = true;
        var generation = ++_generation;
        _scroll.Dispatcher.BeginInvoke(DispatcherPriority.Loaded, new Action(() =>
        {
            if (generation != _generation) return;
            _scroll.UpdateLayout();
            if (_following) _scroll.ScrollToBottom();
            else if (_anchor is {} anchor && _rows.TryGetValue(anchor.Id, out var row))
                _scroll.ScrollToVerticalOffset(_scroll.VerticalOffset + row.View.TranslatePoint(new Point(), _scroll).Y - anchor.Y);
            else _scroll.ScrollToVerticalOffset(_heldOffset);
            _scroll.UpdateLayout();
            _updating = false;
            CaptureAnchor();
        }));
    }
}
