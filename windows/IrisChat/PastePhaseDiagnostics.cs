using System;
using System.Collections.Generic;
using System.Diagnostics;

namespace IrisChat;

// The native harness opts in; normal pastes allocate no timing records and emit
// no diagnostics or clipboard contents.
internal sealed class PastePhaseDiagnostics
{
    internal static Action<IReadOnlyDictionary<string, double>>? Observer { get; set; }
    private readonly Action<IReadOnlyDictionary<string, double>> _observer;
    private readonly Stopwatch _clock = Stopwatch.StartNew();
    private readonly Dictionary<string, double> _phases = new();
    private double _previous;

    private PastePhaseDiagnostics(Action<IReadOnlyDictionary<string, double>> observer) => _observer = observer;
    internal static PastePhaseDiagnostics? Begin() => Observer is {} observer ? new(observer) : null;
    internal void Mark(string phase)
    {
        var now = _clock.Elapsed.TotalMilliseconds;
        _phases.Add(phase, now - _previous);
        _previous = now;
    }
    internal void Complete() => _observer(_phases);
}
