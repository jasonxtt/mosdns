"""Round-7 safety gates. Semantic mismatch alone never requests a stop."""
import collections


class SafetyError(RuntimeError):
    pass


class Safety:
    def __init__(self, started):
        self.started = started
        self.identity = None
        self.not_ready_since = None
        self.cpu = collections.deque()
        self.pairs = collections.deque()
        self.bad_total = 0
        self.rolling_bad = 0

    def health(self, now, props, ready, planned=False):
        identity = (props["MainPID"], props["NRestarts"])
        active = props["ActiveState"] == "active" and int(props["MainPID"]) != 0
        if not active and not planned:
            raise SafetyError("Rust service crash/inactive")
        if self.identity is not None and self.identity[1] != identity[1]:
            raise SafetyError("Rust automatic restart")
        if active and self.identity is not None and self.identity != identity and not planned:
            raise SafetyError("Rust restart/PID change")
        if active:
            self.identity = identity
        if ready and active:
            self.not_ready_since = None
        else:
            if self.not_ready_since is None:
                self.not_ready_since = now
            if now - self.not_ready_since >= 30:
                raise SafetyError("Rust health not ready for 30 seconds")

    def resources(self, now, cpu_seconds, rss_kib, guard):
        if any(guard):
            raise SafetyError("physical leak guard nonzero")
        if rss_kib > 96 * 1024:
            raise SafetyError("tool RSS exceeds 96 MiB")
        self.cpu.append((now, cpu_seconds))
        cutoff = now - 300
        while len(self.cpu) > 2 and self.cpu[1][0] <= cutoff:
            self.cpu.popleft()
        if self.cpu[0][0] > cutoff or len(self.cpu) < 2:
            return None
        a, b = self.cpu[0], self.cpu[1]
        # Interpolate the cumulative cgroup CPU counter at exactly -300s.
        baseline = a[1] + (b[1] - a[1]) * (cutoff - a[0]) / (b[0] - a[0])
        percent = (cpu_seconds - baseline) / 3
        if percent > 10 + 1e-9:
            raise SafetyError("tool CPU exceeds 10% over rolling five minutes")
        return percent

    def pair(self, now, go_rcode, rust_rcode):
        bad = go_rcode == 0 and rust_rcode == 2
        self.bad_total += bad
        self.rolling_bad += bad
        self.pairs.append((now, bad))
        while self.pairs and self.pairs[0][0] < now - 300:
            self.rolling_bad -= self.pairs.popleft()[1]
        count = self.rolling_bad
        snapshot = {"total_bad": self.bad_total, "rolling_paired": len(self.pairs), "rolling_bad": count,
                    "full_window": now - self.started >= 300}
        if self.bad_total > 20:
            raise SafetyError("Rust SERVFAIL versus Go NOERROR total exceeds 20")
        if count * 100 > len(self.pairs):
            raise SafetyError("Rust SERVFAIL versus Go NOERROR exceeds 1% rolling five minutes")
        return snapshot
