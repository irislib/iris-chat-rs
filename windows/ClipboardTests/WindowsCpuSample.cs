using System;
using System.Runtime.InteropServices;

// FILETIME counters are in 100 ns units. The thread pseudo-handle belongs to the
// calling UI thread and must not be closed. Only existing phase boundaries sample.
internal readonly record struct WindowsCpuSample(
    bool ThreadValid, int ThreadError, ulong ThreadKernel, ulong ThreadUser,
    bool SystemValid, int SystemError, ulong SystemIdle, ulong SystemKernel, ulong SystemUser)
{
    internal static WindowsCpuSample Capture()
    {
        var threadValid = GetThreadTimes(GetCurrentThread(), out _, out _, out var kernel, out var user);
        var threadError = threadValid ? 0 : Marshal.GetLastWin32Error();
        var systemValid = GetSystemTimes(out var idle, out var systemKernel, out var systemUser);
        var systemError = systemValid ? 0 : Marshal.GetLastWin32Error();
        return new(threadValid, threadError, kernel.Ticks, user.Ticks,
            systemValid, systemError, idle.Ticks, systemKernel.Ticks, systemUser.Ticks);
    }

    internal object Delta(WindowsCpuSample after)
    {
        var threadValid = ThreadValid && after.ThreadValid &&
            after.ThreadKernel >= ThreadKernel && after.ThreadUser >= ThreadUser;
        var systemValid = SystemValid && after.SystemValid && after.SystemIdle >= SystemIdle &&
            after.SystemKernel >= SystemKernel && after.SystemUser >= SystemUser;
        var idle = systemValid ? (double?)(after.SystemIdle - SystemIdle) / 10_000 : null;
        var kernel = systemValid ? (double?)(after.SystemKernel - SystemKernel) / 10_000 : null;
        var user = systemValid ? (double?)(after.SystemUser - SystemUser) / 10_000 : null;
        var total = kernel + user;
        return new
        {
            ui_thread_valid = threadValid,
            ui_thread_cpu_ms = threadValid ? (double?)((after.ThreadKernel - ThreadKernel) +
                (after.ThreadUser - ThreadUser)) / 10_000 : null,
            ui_thread_errors = new[] { ThreadError, after.ThreadError },
            system_valid = systemValid, system_errors = new[] { SystemError, after.SystemError },
            system_idle_ms = idle, system_kernel_including_idle_ms = kernel, system_user_ms = user,
            system_total_ms = total,
            system_non_idle_valid = systemValid && total >= idle,
            system_non_idle_ms = total >= idle ? total - idle : null,
        };
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct FileTime
    {
        internal uint Low, High;
        internal readonly ulong Ticks => ((ulong)High << 32) | Low;
    }

    [DllImport("kernel32.dll")]
    private static extern IntPtr GetCurrentThread();

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetThreadTimes(IntPtr thread, out FileTime creation, out FileTime exit,
        out FileTime kernel, out FileTime user);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetSystemTimes(out FileTime idle, out FileTime kernel, out FileTime user);
}
