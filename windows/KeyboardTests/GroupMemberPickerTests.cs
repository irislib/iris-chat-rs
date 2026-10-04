using System;
using System.Linq;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Views;

internal static class GroupMemberPickerTests
{
    public static void Run(Window window, AppManager manager)
    {
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic;
        var apply = typeof(AppManager).GetMethod("Apply", flags)!;
        var lastRevision = typeof(AppManager).GetField("_lastRevApplied", flags)!;
        var pendingNavigation = typeof(AppManager).GetField("_pendingNavigationOverride", flags)!;
        var ffi = (FfiApp)typeof(AppManager).GetField("_ffi", flags)!.GetValue(manager)!;
        // Drain the preceding history test's queued navigation before replacing
        // its state. Production navigation reconciliation must not erase this group.
        _ = ffi.ExportSupportBundleJson();
        var settled = ffi.State();
        Until(() => manager.State.rev >= settled.rev && pendingNavigation.GetValue(manager) == null,
            "Previous account actions and navigation did not settle");
        var original = manager.State;
        // This projection fixture still uses the real Apply/notification/view path.
        // Keep asynchronous core snapshots out until its synthetic state is restored.
        ulong revision = 1UL << 63;
        var contacts = Enumerable.Range(0, 21).Select(index => original.chatList.First() with {
            chatId = (index + 1).ToString("x2").PadRight(64, '0'), kind = ChatKind.Direct,
            displayName = $"Person {index}", profileName = $"Person {index}", nickname = null,
            pictureUrl = null, subtitle = null,
        }).ToArray();
        var details = new GroupDetailsSnapshot("picker-test", "People", null, null, "You", "",
            true, false, 1, Array.Empty<GroupMemberSnapshot>());
        void Update(GroupDetailsSnapshot next) {
            details = next;
            apply.Invoke(manager, new object[] { new AppUpdate.FullState(original with {
                rev = ++revision, chatList = contacts, groupDetails = details, currentChat = null,
                router = new Router(original.router.defaultScreen,
                    new Screen[] { new Screen.GroupDetails(details.groupId) }),
            }) });
            Pump();
            Check(manager.State.rev == revision && manager.GroupDetails?.groupId == details.groupId
                && manager.ChatList.Length == contacts.Length,
                $"Picker fixture was replaced: revision={manager.State.rev}/{revision}, "
                + $"group={manager.GroupDetails?.groupId}, contacts={manager.ChatList.Length}/{contacts.Length}");
        }
        try
        {
            Update(details);
            var view = new GroupDetailsView { GroupId = details.groupId };
            window.Content = view;
            Until(() => view.IsLoaded, "Group details view did not load");
            var list = (ItemsControl)view.FindName("KnownUsersList");
            var scroll = (ScrollViewer)view.FindName("KnownUsersScroll");
            var input = (TextBox)view.FindName("AddMemberInput");
            Check(list.Items.Count == 21,
                $"All contacts beyond the old eight-row cutoff are rendered: actual={list.Items.Count}, expected=21, loaded={view.IsLoaded}");
            window.UpdateLayout();
            Check(scroll.ActualHeight <= 360 && scroll.ScrollableHeight > 0, "Candidates use a bounded scroll viewport");
            scroll.ScrollToEnd(); Pump();
            Check(scroll.VerticalOffset > 0, "The last candidate can be reached by scrolling");
            foreach (int index in new[] { 19, 17 }) {
                var row = (Border)list.Items.Cast<Border>().Single(item => {
                    var grid = (Grid)item.Child;
                    return grid.Children.OfType<IrisChat.Chrome.Avatar>().Single().OwnerPubkeyHex == contacts[index].chatId;
                });
                var check = ((Grid)row.Child).Children.OfType<CheckBox>().Single();
                check.RaiseEvent(new RoutedEventArgs(CheckBox.ClickEvent)); Pump();
                Check(((Button)view.FindName("AddMemberButton")).IsEnabled, "A later candidate can be selected");
                Update(details with { members = details.members.Append(new GroupMemberSnapshot(
                    null, contacts[index].chatId, $"Person {index}", "", null, false, false, false)).ToArray() });
                Check(list.Items.Count == 21 - details.members.Length, "Fresh membership immediately removes each added contact");
                Check(!((Button)view.FindName("AddMemberButton")).IsEnabled, "A newly added contact is no longer selected");
            }
            input.Text = "Person 20"; Pump();
            Check(list.Items.Count == 1, "Searching still reaches contacts after the old cutoff");
            contacts[20] = contacts[20] with { displayName = "Updated contact", profileName = "Updated contact" };
            Update(details);
            Check(list.Items.Count == 0, "A live profile change removes the stale search match");
            input.Text = "Updated contact"; Pump();
            Check(list.Items.Count == 1, "A live profile change is searchable without reopening the picker");
            Update(details with { canManage = false });
            Check(list.Items.Count == 0 && !input.IsEnabled, "Losing group authorization closes member choices");
            Console.WriteLine("PASS: WPF full member candidates, bounded scrolling, sequential membership/profile refresh and authorization");
        }
        finally
        {
            window.Content = null;
            lastRevision.SetValue(manager, 0UL);
            apply.Invoke(manager, new object[] { new AppUpdate.FullState(original) });
        }
    }

    private static void Until(Func<bool> ready, string message) {
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (!ready()) {
            Check(DateTime.UtcNow < deadline, message);
            Pump();
            System.Threading.Thread.Sleep(5);
        }
    }
    private static void Pump() {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }
    private static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
}
