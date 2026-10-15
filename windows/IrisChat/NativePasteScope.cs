using System;
using System.Runtime.CompilerServices;
using System.Windows.Controls;

namespace IrisChat;

/// Completes a reentrant draft replacement after native Paste closes its undo unit.
internal sealed class NativePasteScope : IDisposable
{
    private static readonly ConditionalWeakTable<TextBox, NativePasteScope> Active = new();
    private readonly TextBox _input;
    private readonly bool _readOnly;
    private Action? _restore;
    private int _depth = 1;

    private NativePasteScope(TextBox input)
    {
        _input = input;
        _readOnly = input.IsReadOnly;
        Active.Add(input, this);
    }

    internal static NativePasteScope Begin(TextBox input)
    {
        if (!Active.TryGetValue(input, out var scope)) return new(input);
        scope._depth++;
        return scope;
    }

    internal static bool DeferRestore(TextBox input, Action restore)
    {
        if (!Active.TryGetValue(input, out var scope)) return false;
        // The latest requested draft wins. Until it can be installed, prevent
        // input from being accepted into the obsolete editor contents.
        scope._restore = restore;
        input.IsReadOnly = true;
        return true;
    }

    public void Dispose()
    {
        if (--_depth > 0) return;
        Active.Remove(_input);
        if (_restore == null) return;
        _input.IsReadOnly = _readOnly;
        _restore();
    }
}
