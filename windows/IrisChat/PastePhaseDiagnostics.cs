using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Windows.Controls;

namespace IrisChat;

// The native harness opts in; normal pastes allocate no timing records and emit
// no diagnostics or clipboard contents.
internal sealed class PastePhaseDiagnostics
{
    internal static Action<IReadOnlyDictionary<string, double>>? Observer { get; set; }
    [ThreadStatic] private static PastePhaseDiagnostics? _current;
    private readonly PastePhaseDiagnostics? _parent;
    private readonly TextBox _input;
    private readonly Action<IReadOnlyDictionary<string, double>> _observer;
    private readonly Stopwatch _clock = Stopwatch.StartNew();
    private readonly Dictionary<string, double> _phases = new();
    private double _previous;
    private double _nativeStarted;
    private bool _textChanged;
    private bool _payloadReady;

    private PastePhaseDiagnostics(TextBox input, Action<IReadOnlyDictionary<string, double>> observer)
    {
        _input = input;
        _observer = observer;
        _parent = _current;
        _current = this;
    }
    internal static PastePhaseDiagnostics? Begin(TextBox input) => Observer is {} observer ? new(input, observer) : null;
    internal static PastePhaseDiagnostics? For(TextBox input) => _current?._input == input ? _current : null;
    internal void BeginNativePaste() => _nativeStarted = _clock.Elapsed.TotalMilliseconds;
    internal void TextChangedStarted()
    {
        _textChanged = true;
        Mark(_payloadReady ? "native_payload_to_text_changed_notification" : "native_pending_input_to_text_changed_notification");
    }
    internal void PayloadMaterialized()
    {
        _payloadReady = true;
        Mark("guarded_payload_materialization");
    }
    internal void NativePasteReturned()
    {
        var nativeMs = _clock.Elapsed.TotalMilliseconds - _nativeStarted;
        Mark(_textChanged ? "native_completion_after_text_changed" : "native_completion_without_text_changed");
        _phases.Add("native_paste_total", nativeMs);
    }
    internal void Mark(string phase)
    {
        var now = _clock.Elapsed.TotalMilliseconds;
        var key = phase;
        for (var occurrence = 2; _phases.ContainsKey(key); occurrence++) key = $"{phase}_{occurrence}";
        _phases.Add(key, now - _previous);
        _previous = now;
    }
    internal void Complete()
    {
        _current = _parent;
        _observer(_phases);
    }
}
