using System;
using System.Linq;
using System.Collections.Generic;
using System.Windows.Input;
using System.Windows.Controls;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class MentionEditingTests
{
    internal static void Run(ComposerBar composer, TextBox input, string first, string second,
        Func<string> saved, Action pump)
    {
        var one = Native.FormatMessageMention(first, "Alex");
        var two = Native.FormatMessageMention(second, "Alex");

        composer.RestoreDraft(one + " " + two);
        input.Select(0, "@Alex ".Length);
        input.SelectedText = ""; pump();
        Identities(saved(), second);
        Check(input.Text == "@Alex", "Deleting the first duplicate label keeps the second visible label");
        input.Undo(); pump();
        Identities(saved(), first, second);
        input.Redo(); pump();
        Identities(saved(), second);

        composer.RestoreDraft(one + " " + two);
        input.Select("@Alex ".Length, "@Alex".Length);
        input.SelectedText = ""; pump();
        Identities(saved(), first);
        input.Undo(); pump();
        Identities(saved(), first, second);

        // The same visible text must still lose its identity when it was
        // explicitly replaced with ordinary text, and regain it on undo.
        composer.RestoreDraft(one + " " + two);
        input.Select(0, "@Alex".Length);
        input.SelectedText = "@Alex"; pump();
        Identities(saved(), second);
        input.Undo(); pump();
        Identities(saved(), first, second);
        input.Redo(); pump();
        Identities(saved(), second);

        composer.RestoreDraft("👋 " + one + " " + two);
        input.Select("👋 @Al".Length, 1);
        input.SelectedText = "i"; pump();
        Identities(saved(), second);
        input.Undo(); pump();
        Identities(saved(), first, second);
        input.Redo(); pump();
        Identities(saved(), second);

        // TextChange offsets describe the final buffer. Applying them in
        // ascending order must preserve the middle span through a batched edit.
        composer.RestoreDraft(one + " / " + two);
        input.BeginChange();
        try
        {
            input.Select(0, 0); input.SelectedText = "Hi ";
            // TextBox.Text is deferred until EndChange; use the native buffer's
            // intended coordinate instead of reading that stale property.
            input.Select("Hi @Alex / ".Length, "@Alex".Length);
            input.SelectedText = "there";
        }
        finally { input.EndChange(); }
        pump();
        Check(input.Text == "Hi @Alex / there", "Batched edit targets the second duplicate label");
        Identities(saved(), first);
        input.Undo(); pump();
        Identities(saved(), first, second);
        input.Redo(); pump();
        Identities(saved(), first);

        // Real text-input events reopen the typing undo unit. The sidecar must
        // keep its original before-state while updating the merged after-state.
        composer.RestoreDraft(one);
        input.Focus(); input.Select("@Al".Length, 0);
        var actions = new List<UndoAction>();
        TextChangedEventHandler recordAction = (_, e) => actions.Add(e.UndoAction);
        input.TextChanged += recordAction;
        try
        {
            Type(input, "x"); pump();
            Type(input, "y"); pump();
        }
        finally { input.TextChanged -= recordAction; }
        Check(actions.Contains(UndoAction.Merge), "Consecutive typing exercises a merged native undo unit");
        Identities(saved());
        input.Undo(); pump();
        Check(input.Text == "@Alex", "One undo reverses the merged typing unit");
        Identities(saved(), first);
        input.Redo(); pump();
        Identities(saved());

        // Loading another draft also starts a fresh native undo history.
        composer.RestoreDraft(two);
        Check(!input.CanUndo && !input.CanRedo, "Restored drafts cannot replay another draft's history");
        input.Select(input.Text.Length, 0); input.SelectedText = "!"; pump();
        Identities(saved(), second);
        input.Undo(); pump();
        Identities(saved(), second);
        Check(!input.CanUndo, "Undo stops at the restored draft");
        Console.WriteLine("PASS: duplicate mention identities, actual edit ranges, undo/redo and restored-draft history");
    }

    private static void Type(TextBox input, string text) => input.RaiseEvent(new TextCompositionEventArgs(
        Keyboard.PrimaryDevice, new TextComposition(InputManager.Current, input, text)) {
        RoutedEvent = TextCompositionManager.TextInputEvent,
    });

    internal static void Identities(string wire, params string[] expected)
    {
        var actual = Native.MessageMentions(wire).Select(mention => mention.ownerPubkeyHex).ToArray();
        Check(actual.SequenceEqual(expected),
            "Mention identities follow the actual editor operation; expected " + string.Join(",", expected)
                + "; got " + string.Join(",", actual) + "; wire: " + wire);
    }

    private static void Check(bool passed, string message) { if (!passed) throw new Exception(message); }
}
