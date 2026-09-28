using System;
using System.Diagnostics;
using NAudio.Wave;

namespace IrisChat;

internal static class AudioWaveform
{
    public const int BarCount = 47;

    // Media Foundation decodes without playing sound or requiring an output
    // device. Keep only 47 peaks, even for long attachments.
    public static float[] Decode(string path)
    {
        try
        {
            using var reader = new MediaFoundationReader(path);
            var source = reader.ToSampleProvider();
            var totalSamples = reader.Length / (reader.WaveFormat.BitsPerSample / 8);
            if (totalSamples <= 0) return Array.Empty<float>();
            var sums = new double[BarCount];
            var counts = new long[BarCount];
            var buffer = new float[8192];
            var timer = Stopwatch.StartNew();
            long offset = 0;
            int count;
            while ((count = source.Read(buffer, 0, buffer.Length)) > 0)
            {
                // Slow/unsupported analysis must not prevent playing audio.
                if (timer.Elapsed > TimeSpan.FromSeconds(3)) return Array.Empty<float>();
                for (var i = 0; i < count; i++, offset++)
                {
                    var bin = (int)Math.Min(BarCount - 1, offset * BarCount / totalSamples);
                    var value = Math.Abs(buffer[i]);
                    if (float.IsFinite(value)) { sums[bin] += (double)value * value; counts[bin]++; }
                }
            }
            if (offset == 0) return Array.Empty<float>();
            var peaks = new float[BarCount];
            for (var i = 0; i < peaks.Length; i++) peaks[i] = (float)Math.Sqrt(sums[i] / Math.Max(1, counts[i]));
            var maximum = 0f;
            foreach (var peak in peaks) maximum = Math.Max(maximum, peak);
            if (maximum > 0.0001f)
                for (var i = 0; i < peaks.Length; i++) peaks[i] /= maximum;
            return peaks;
        }
        catch (Exception error)
        {
            Trace.TraceWarning("Audio waveform unavailable: {0}", error.Message);
            return Array.Empty<float>();
        }
    }
}
