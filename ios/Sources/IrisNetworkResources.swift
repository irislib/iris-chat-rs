import Darwin

enum IrisNetworkResources {
    /// Keep separate anonymous discovery connections without using the small
    /// launchd default as a reason to reconnect on every quiet poll. This only
    /// adjusts our process's soft limit; the core independently retains at most
    /// 512 sockets and uses no more than a quarter of the resulting limit.
    static func prepare(
        readLimit: (inout rlimit) -> Int32 = { getrlimit(RLIMIT_NOFILE, &$0) },
        writeLimit: (inout rlimit) -> Int32 = { setrlimit(RLIMIT_NOFILE, &$0) }
    ) {
        var current = rlimit()
        guard readLimit(&current) == 0 else { return }
        let desired = min(rlim_t(2048), current.rlim_max)
        guard current.rlim_cur < desired else { return }
        current.rlim_cur = desired
        // Refusal leaves the existing OS limit in place. The core queries that
        // actual limit and uses the smaller retention budget automatically.
        _ = writeLimit(&current)
    }
}
