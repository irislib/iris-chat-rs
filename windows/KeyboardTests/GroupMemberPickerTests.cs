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
        var original = manager.State;
        var apply = typeof(AppManager).GetMethod("Apply", BindingFlags.Instance | BindingFlags.NonPublic)!;
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
                rev = manager.State.rev + 1, chatList = contacts, groupDetails = details,
            }) });
            Pump();
        }
        Update(details);
        var view = new GroupDetailsView { GroupId = details.groupId };
        window.Content = view; Pump();
        var list = (ItemsControl)view.FindName("KnownUsersList");
        var scroll = (ScrollViewer)view.FindName("KnownUsersScroll");
        var input = (TextBox)view.FindName("AddMemberInput");
        Check(list.Items.Count == 21, "All contacts beyond the old eight-row cutoff are rendered");
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
        Update(details with { canManage = false });
        Check(list.Items.Count == 0 && !input.IsEnabled, "Losing group authorization closes member choices");
        window.Content = null;
        apply.Invoke(manager, new object[] { new AppUpdate.FullState(original with { rev = manager.State.rev + 1 }) });
        Console.WriteLine("PASS: WPF full member candidates, bounded scrolling, sequential membership refresh and authorization");
    }

    private static void Pump() {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.ApplicationIdle, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }
    private static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
}
