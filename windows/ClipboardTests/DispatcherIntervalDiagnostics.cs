using System;
using System.Diagnostics;
using System.Threading;
using System.Windows.Threading;

// Diagnostic-only observer. No dispatcher operations, logging, content/delegate
// inspection, CPU queries, or growing collections inside the hook callbacks.
internal sealed class DispatcherIntervalDiagnostics : IDisposable
{
    private const int Capacity = 256;
    private readonly DispatcherHooks hooks = Dispatcher.CurrentDispatcher.Hooks;
    private readonly int ownerThread = Environment.CurrentManagedThreadId;
    private readonly Entry[] entries = new Entry[Capacity];
    private bool active;
    private bool attached;
    private long start, end, observerTicks;
    private int count, marker, droppedStarts, unmatchedCompletions, duplicateStarts;
    private int nonLifoCompletions, foreignThreadCallbacks, startCallbacks, completionCallbacks;

    private struct Entry
    {
        internal DispatcherOperation? Operation;
        internal long Start, End, StartObserverTicks, EndObserverTicks;
        internal int StartPriority, EndPriority, Parent, Depth, StartMarker, EndMarker;
        internal DispatcherOperationStatus EndStatus;
    }

    internal void Begin(long timestamp)
    {
        start = timestamp;
        Volatile.Write(ref active, true);
        hooks.OperationStarted += OnStarted;
        hooks.OperationCompleted += OnCompleted;
        attached = true;
    }

    internal void Mark(int index) => marker = index;

    internal void End(long timestamp)
    {
        end = timestamp;
        Dispose();
    }

    private void OnStarted(object sender, DispatcherHookEventArgs args)
    {
        if (!Volatile.Read(ref active)) return;
        if (Environment.CurrentManagedThreadId != ownerThread)
        {
            Interlocked.Increment(ref foreignThreadCallbacks);
            return;
        }
        var timestamp = Stopwatch.GetTimestamp();
        startCallbacks++;
        var index = -1;
        if (Find(args.Operation) >= 0) duplicateStarts++;
        else if (count == Capacity) droppedStarts++;
        else
        {
            var parent = LastOpen();
            index = count++;
            entries[index] = new Entry
            {
                Operation = args.Operation, Start = timestamp,
                StartPriority = (int)args.Operation.Priority, Parent = parent,
                Depth = parent < 0 ? 0 : entries[parent].Depth + 1, StartMarker = marker,
            };
        }
        var cost = Stopwatch.GetTimestamp() - timestamp;
        observerTicks += cost;
        if (index >= 0) entries[index].StartObserverTicks = cost;
    }

    private void OnCompleted(object sender, DispatcherHookEventArgs args)
    {
        if (!Volatile.Read(ref active)) return;
        if (Environment.CurrentManagedThreadId != ownerThread)
        {
            Interlocked.Increment(ref foreignThreadCallbacks);
            return;
        }
        var timestamp = Stopwatch.GetTimestamp();
        completionCallbacks++;
        var index = Find(args.Operation);
        if (index < 0) unmatchedCompletions++;
        else
        {
            // Reference pairing remains correct if nested dispatch occurs.
            // Unexpected completion order is reported, never repaired/inferred.
            if (index != LastOpen()) nonLifoCompletions++;
            entries[index].End = timestamp;
            entries[index].EndPriority = (int)args.Operation.Priority;
            entries[index].EndStatus = args.Operation.Status;
            entries[index].EndMarker = marker;
            entries[index].Operation = null;
        }
        var cost = Stopwatch.GetTimestamp() - timestamp;
        observerTicks += cost;
        if (index >= 0) entries[index].EndObserverTicks = cost;
    }

    private int Find(DispatcherOperation operation)
    {
        for (var i = count - 1; i >= 0; i--)
            if (ReferenceEquals(entries[i].Operation, operation)) return i;
        return -1;
    }

    private int LastOpen()
    {
        for (var i = count - 1; i >= 0; i--)
            if (entries[i].Operation != null) return i;
        return -1;
    }

    internal object Report()
    {
        var rows = new object[count];
        var unfinished = 0;
        for (var i = 0; i < count; i++)
        {
            var entry = entries[i];
            var completed = entry.End != 0;
            if (!completed) unfinished++;
            rows[i] = new
            {
                id = i, parent_id = entry.Parent < 0 ? (int?)null : entry.Parent,
                observed_depth = entry.Depth, start_after_marker = entry.StartMarker,
                end_after_marker = completed ? (int?)entry.EndMarker : null,
                start_ms = Milliseconds(entry.Start - start),
                end_ms = completed ? (double?)Milliseconds(entry.End - start) : null,
                inclusive_wall_ms = completed ? (double?)Milliseconds(entry.End - entry.Start) : null,
                start_priority = ((DispatcherPriority)entry.StartPriority).ToString(),
                end_priority = completed ? ((DispatcherPriority)entry.EndPriority).ToString() : null,
                status_at_completed_hook = completed ? entry.EndStatus.ToString() : null,
                completed,
                start_observer_ms = Milliseconds(entry.StartObserverTicks),
                completion_observer_ms = completed ? (double?)Milliseconds(entry.EndObserverTicks) : null,
            };
        }
        var foreign = Volatile.Read(ref foreignThreadCallbacks);
        var complete = end != 0 && droppedStarts == 0 && unmatchedCompletions == 0 &&
            duplicateStarts == 0 && nonLifoCompletions == 0 && foreign == 0 && unfinished == 0;
        return new
        {
            capacity = Capacity, overflow = droppedStarts != 0,
            complete, incomplete = !complete,
            start_callbacks = startCallbacks, completion_callbacks = completionCallbacks,
            dropped_starts = droppedStarts, unmatched_completions = unmatchedCompletions,
            duplicate_starts = duplicateStarts, non_lifo_completions = nonLifoCompletions,
            foreign_thread_callbacks = foreign, unfinished_operations = unfinished,
            observed_interval_ms = end == 0 ? (double?)null : Milliseconds(end - start),
            callback_body_observer_ms = Milliseconds(observerTicks), operations = rows,
            boundary = "Queued Started/Completed hooks only. Inclusive times include nested work; do not sum nested rows. Completed is a hook, not proof of task success. Same-thread Send, native messages and inline task completions are not covered. Uncovered time is not proof of idle or scheduler pressure. Observer cost excludes WPF event-argument allocation, hook dispatch, thread checks and JIT; no cost is subtracted from the test budget.",
        };
    }

    private static double Milliseconds(long ticks) => ticks * 1000.0 / Stopwatch.Frequency;

    public void Dispose()
    {
        Volatile.Write(ref active, false);
        if (!attached) return;
        hooks.OperationStarted -= OnStarted;
        hooks.OperationCompleted -= OnCompleted;
        attached = false;
    }
}
