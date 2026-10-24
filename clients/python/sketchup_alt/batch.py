"""Batch builder and batch references."""
import json
import re

_KINDS = "vefigrnsmdy"


class RefId(str):
    """A reference string such as "$0:f0", usable wherever the API takes an id."""


class Ref:
    """Stands for the entities command ``index`` of a batch made.

    ``Ref(0).f(0)`` is ``"$0:f0"``, ``Ref(0).e("*")`` is ``"$0:e*"`` and
    ``Ref(0).all()`` is ``"$0:*"``. Kinds: v vertex, e edge, f face, i instance,
    g guide, r dimension, n note, s section plane, m material, d definition,
    y style.
    """

    def __init__(self, index):
        if not isinstance(index, int) or isinstance(index, bool) or index < 0:
            raise ValueError("a command index is an integer from 0, got %r" % (index,))
        self.index = index

    def _kind(self, kind, n):
        if n != "*" and (not isinstance(n, int) or isinstance(n, bool) or n < 0):
            raise ValueError('an index is an integer from 0 or "*", got %r' % (n,))
        return RefId("$%d:%s%s" % (self.index, kind, n))

    def all(self):
        return RefId("$%d:*" % self.index)

    def __getattr__(self, name):
        if len(name) == 1 and name in _KINDS:
            return lambda n=0: self._kind(name, n)
        raise AttributeError(name)

    def __repr__(self):
        return "Ref(%d)" % self.index


def _default(o):
    if isinstance(o, Ref):
        raise TypeError("a bare Ref is not a value; use ref.f(0), ref.all() and so on")
    raise TypeError("not JSON serializable: %r" % (o,))


def _snake_ok(name):
    return re.fullmatch(r"[a-z][a-z0-9_]*", name) is not None


class Batch:
    """Collects commands; leaving the ``with`` block sends them as one batch.

    Every command in the app's ``describe`` schema is a method named like its
    ``type`` (``add_rectangle``, ``push_pull``...); each returns a ``Ref`` to
    that command's results. Fields go by keyword; an unknown field or a missing
    required one raises ``TypeError`` before anything is sent. After the block
    ``result`` holds the batch result (``outcomes``, ``changes``, ``revision``).
    """

    def __init__(self, client, atomic=True):
        self._client = client
        self._schema = client.command_schema()
        self.atomic = atomic
        self.commands = []
        self.result = None

    def command(self, type, **fields):
        """Add the command ``type`` with ``fields``; returns its ``Ref``."""
        spec = self._schema.get(type)
        if spec is None:
            raise AttributeError("no command named %r" % type)
        unknown = sorted(set(fields) - spec["fields"])
        if unknown:
            raise TypeError("%s got unknown field(s) %s; fields are %s" % (type, ", ".join(unknown), ", ".join(sorted(spec["fields"]))))
        missing = sorted(spec["required"] - set(fields))
        if missing:
            raise TypeError("%s is missing required field(s) %s" % (type, ", ".join(missing)))
        cmd = {"type": type}
        cmd.update(fields)
        json.dumps(cmd, default=_default)  # fail on a bad value before sending
        self.commands.append(cmd)
        return Ref(len(self.commands) - 1)

    def __getattr__(self, name):
        if name.startswith("_") or "_schema" not in self.__dict__:
            raise AttributeError(name)
        if name in self._schema:
            def method(**fields):
                return self.command(name, **fields)

            method.__name__ = name
            method.__doc__ = self._schema[name]["doc"]
            return method
        raise AttributeError("no command named %r" % name)

    def __dir__(self):
        return sorted(set(super().__dir__()) | set(self._schema))

    def send(self):
        """Send the collected commands now; returns the batch result."""
        self.result = self._client.batch(self.commands, atomic=self.atomic)
        return self.result

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        if exc_type is None:
            self.send()
        return False

    # Hand-written helpers for common commands (documented; same checks apply).

    def add_rectangle(self, corner, opposite, **opts):
        """Draw an axis-aligned rectangle from ``corner`` to ``opposite`` (model points)."""
        return self.command("add_rectangle", corner=corner, opposite=opposite, **opts)

    def add_circle(self, center, radius, **opts):
        """Draw a circle of ``radius`` around ``center``; ``segments`` and ``normal`` are optional."""
        return self.command("add_circle", center=center, radius=radius, **opts)

    def push_pull(self, face, distance, **opts):
        """Push or pull ``face`` (an id or reference such as ``ref.f(0)``) by ``distance`` meters."""
        return self.command("push_pull", face=face, distance=distance, **opts)

    def add_edge(self, start, end, **opts):
        """Draw a straight edge from ``start`` to ``end``."""
        return self.command("add_edge", start=start, end=end, **opts)
