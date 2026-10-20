"""Decode and project JSON array entries without retaining unused API rows."""
import json


class Reader:
    def __init__(self, stream):
        self.stream, self.buffer, self.eof = stream, '', False
        self.decoder = json.JSONDecoder()

    def fill(self):
        chunk = self.stream.read(4096)
        self.eof = not chunk
        self.buffer += chunk
        if len(self.buffer) > 2 * 1024 * 1024:
            raise ValueError('single JSON value exceeds projection budget')

    def peek(self):
        while True:
            self.buffer = self.buffer.lstrip()
            if self.buffer or self.eof:
                return self.buffer[:1]
            self.fill()

    def take(self, wanted):
        if self.peek() != wanted:
            raise ValueError('unexpected JSON delimiter')
        self.buffer = self.buffer[1:]

    def value(self):
        self.peek()
        while True:
            try:
                value, end = self.decoder.raw_decode(self.buffer)
                # A scalar may be split across reads (e.g. 12 | 345).
                if end == len(self.buffer) and not self.eof:
                    self.fill()
                    continue
                if end < len(self.buffer) and self.buffer[end] not in ',:]} \n\r\t':
                    if self.eof:
                        raise ValueError('invalid JSON scalar')
                    self.fill()
                    continue
                self.buffer = self.buffer[end:]
                return value
            except json.JSONDecodeError:
                if self.eof:
                    raise ValueError('incomplete JSON value') from None
                self.fill()

    def array(self, project):
        self.take('[')
        result = []
        if self.peek() != ']':
            while True:
                item = project(self.value())
                if item is not None:
                    result.append(item)
                if self.peek() == ']':
                    break
                self.take(',')
        self.take(']')
        return result


def project_json(stream, project, member=None):
    reader = Reader(stream)
    if member is None:
        result = reader.array(project)
    else:
        reader.take('{')
        result = None
        if reader.peek() != '}':
            while True:
                key = reader.value()
                reader.take(':')
                if key == member:
                    if result is not None:
                        raise ValueError('duplicate array member')
                    result = reader.array(project)
                else:
                    reader.value()
                if reader.peek() == '}':
                    break
                reader.take(',')
        reader.take('}')
        if result is None:
            raise ValueError('array member missing')
    if reader.peek():
        raise ValueError('trailing JSON data')
    return result
