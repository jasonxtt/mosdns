"""Ten-minute learned-file safety net; emits counts, never learned keys."""
import json
import pathlib


class Counts:
    def __init__(self, files, output):
        self.files = files
        self.output = output
        self.previous = {}

    def sample(self, wall):
        counts = {}
        for tag, filename in self.files.items():
            try:
                with pathlib.Path(filename).open() as stream:
                    counts[tag] = sum(len(line.split()) >= 2 for line in stream)
            except FileNotFoundError:
                counts[tag] = 0
        decreased = [tag for tag, count in counts.items() if count < self.previous.get(tag, 0)]
        row = {"wall": wall, "counts": counts, "decreased": decreased}
        with self.output.open("a") as stream:
            stream.write(json.dumps(row) + "\n")
        # No automatic waiver. A documented flush must be recorded/verified
        # by the operator before a subsequent authorized run can continue.
        if decreased:
            raise RuntimeError("learned stat entry count decrease: possible lifecycle defect; stop")
        self.previous = counts
        return row
