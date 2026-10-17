using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Windows.Controls;
using System.Windows.Media;

// Temporary diagnostic fixture only. Observe the existing edit operations;
// never force layout, pump the dispatcher, or change the performance budgets.
internal sealed class EditPhaseDiagnostics : IDisposable
{
    private readonly Process process = Process.GetCurrentProcess();
    private readonly Sample[] samples = new Sample[9];
    private int count;

    private readonly record struct Sample(string Phase, long Timestamp, double CpuMs,
        long UiThreadAllocatedBytes, int Gen0, int Gen1, int Gen2, long ObserverTicks);

    internal void Mark(string phase)
    {
        if (count == samples.Length) throw new InvalidOperationException("Unexpected edit diagnostic phase");
        var timestamp = Stopwatch.GetTimestamp();
        var cpuMs = process.TotalProcessorTime.TotalMilliseconds;
        var allocated = GC.GetAllocatedBytesForCurrentThread();
        var gen0 = GC.CollectionCount(0);
        var gen1 = GC.CollectionCount(1);
        var gen2 = GC.CollectionCount(2);
        var observerTicks = Stopwatch.GetTimestamp() - timestamp;
        samples[count++] = new(phase, timestamp, cpuMs, allocated, gen0, gen1, gen2, observerTicks);
    }

    internal object Report(TextBox input)
    {
        if (count != samples.Length) throw new InvalidOperationException("Incomplete edit diagnostic phases");
        var phases = new object[count - 1];
        for (var i = 1; i < count; i++)
        {
            var before = samples[i - 1];
            var after = samples[i];
            phases[i - 1] = new
            {
                phase = after.Phase,
                wall_ms = Milliseconds(after.Timestamp - before.Timestamp),
                process_cpu_ms = after.CpuMs - before.CpuMs,
                ui_thread_allocated_bytes = after.UiThreadAllocatedBytes - before.UiThreadAllocatedBytes,
                gc_collections = new[] { after.Gen0 - before.Gen0, after.Gen1 - before.Gen1, after.Gen2 - before.Gen2 },
                preceding_observer_ms = Milliseconds(before.ObserverTicks),
            };
        }
        return new
        {
            diagnostic_only = true,
            runtime = RuntimeInformation.FrameworkDescription,
            os = RuntimeInformation.OSDescription,
            architecture = RuntimeInformation.ProcessArchitecture.ToString(),
            processor_count = Environment.ProcessorCount,
            wpf_assembly_version = typeof(TextBox).Assembly.GetName().Version?.ToString(),
            wpf_file_version = FileVersionInfo.GetVersionInfo(typeof(TextBox).Assembly.Location).FileVersion,
            rendering_tier = RenderCapability.Tier >> 16,
            editor_width = input.ActualWidth,
            editor_height = input.ActualHeight,
            font_size = input.FontSize,
            wrapping = input.TextWrapping.ToString(),
            phases,
            final_observer_ms = Milliseconds(samples[count - 1].ObserverTicks),
            boundary = "Existing post-paste edit operations only; wall phases include preceding observer cost. CPU is aggregate process time, allocations are UI-thread bytes, GC counts are process-wide. No host CPU pressure or display scanout is measured.",
        };
    }

    private static double Milliseconds(long ticks) => ticks * 1000.0 / Stopwatch.Frequency;
    public void Dispose() => process.Dispose();
}
