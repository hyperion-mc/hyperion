#!/usr/bin/env python3
"""A wire assertion must be its own wait.

The class of bug this refuses is the one that shipped as
`smash-selector-e2e` failing with "the action bar never said so: []" while
the server had sent the line 0.2 ms earlier (CI run 30500640846). The check
waited on `client.kit`, which is set from the chat packet, and then asserted
on `client.action_bar`, which is the packet after it. `drain` takes one
`recv` per pump, so a burst split across two reads satisfies the wait with
the asserted-on packet still in flight.

That reads exactly like a server defect and sends somebody into the wrong
code for an afternoon, which is what makes it worth a check of its own
rather than a note in review. The rule:

    an assertion about a field the packet handler fills must be preceded,
    in the same function, by a wait whose predicate reads that same field.

A field this file has never heard of is ignored, and a field is only under
the rule when a packet handler is what fills it, because those are exactly
the fields with a "has not arrived yet" state.

The escape hatch is a comment on the failing line or the line above:

    # not-a-wire-assertion: <why this is not about a packet>

with the reason required. A bare marker is a mute button, so the reason is
part of the syntax and an empty one is itself an error.
"""

from __future__ import annotations

import ast
import pathlib
import re
import sys

# Methods that take a predicate and poll it. The predicate is the first
# argument; nothing here cares about the timeout.
WAITERS = frozenset({"wait_until", "must_become", "must_not_become"})

# Where packets are decoded. A field is "observed" if one of these fills it,
# which is derived rather than listed: a new packet handler brings its field
# under the rule the day it lands, and a renamed field cannot leave a stale
# entry behind.
HANDLERS = frozenset({"handle", "on_chat", "dispatch", "consume"})

MUTATORS = frozenset({"append", "extend", "update", "add", "setdefault", "clear"})

MARKER = re.compile(r"#\s*not-a-wire-assertion:(?P<reason>.*)$")


def attribute_names(node):
    """Every `<anything>.name` read anywhere under `node`."""
    return {n.attr for n in ast.walk(node) if isinstance(n, ast.Attribute)}


def self_calls(node):
    """Every `self.method(...)` called under `node`."""
    out = set()
    for call in ast.walk(node):
        if not isinstance(call, ast.Call):
            continue
        func = call.func
        if (
            isinstance(func, ast.Attribute)
            and isinstance(func.value, ast.Name)
            and func.value.id == "self"
        ):
            out.add(func.attr)
    return out


def observed_fields(tree):
    """The fields a packet handler in this file fills."""
    found = set()
    for fn in ast.walk(tree):
        if not isinstance(fn, ast.FunctionDef) or fn.name not in HANDLERS:
            continue
        for node in ast.walk(fn):
            if (
                isinstance(node, ast.Call)
                and isinstance(node.func, ast.Attribute)
                and node.func.attr in MUTATORS
                and isinstance(node.func.value, ast.Attribute)
            ):
                found.add(node.func.value.attr)
            if isinstance(node, (ast.Assign, ast.AugAssign)):
                targets = node.targets if isinstance(node, ast.Assign) else [node.target]
                for target in targets:
                    if isinstance(target, ast.Attribute):
                        found.add(target.attr)
                    elif isinstance(target, ast.Subscript) and isinstance(
                        target.value, ast.Attribute
                    ):
                        found.add(target.value.attr)
    return found

def waits_of(fn, observed):
    """Observed fields the waits written inside `fn` poll on."""
    waited = set()
    for call in ast.walk(fn):
        if (
            isinstance(call, ast.Call)
            and isinstance(call.func, ast.Attribute)
            and call.func.attr in WAITERS
            and call.args
        ):
            waited |= attribute_names(call.args[0]) & observed
    return waited


def summarise(tree, observed):
    """For each method, the fields its own waits cover, to a fixpoint.

    Waits propagate through `self` calls and reads deliberately do not. A
    helper that waits, like `ask_podiums`, has to credit its caller, or every
    function that reads an answer somebody else waited for is a false report.
    Reads travelling the same edges is what makes the analysis useless: every
    wait reaches `pump` reaches `handle`, which reads every field there is, so
    propagating reads would make every assertion depend on everything.

    The cost of not propagating reads is real and worth naming: an assertion
    whose field is reached only through a helper is not seen by this rule. It
    catches the shape where the field is in front of you, which is the shape
    that shipped.
    """
    methods = {
        fn.name: fn for fn in ast.walk(tree) if isinstance(fn, ast.FunctionDef)
    }
    direct = {name: waits_of(fn, observed) for name, fn in methods.items()}
    calls = {name: self_calls(fn) & set(methods) for name, fn in methods.items()}

    waits = {name: set(value) for name, value in direct.items()}
    changed = True
    while changed:
        changed = False
        for name in methods:
            grown = set(direct[name])
            for callee in calls[name]:
                if callee != name:
                    grown |= waits[callee]
            if grown != waits[name]:
                waits[name] = grown
                changed = True
    return waits, methods


def is_failure(call):
    """`self.fail(...)`, or an append onto something named for failures."""
    if not isinstance(call, ast.Call) or not isinstance(call.func, ast.Attribute):
        return False
    if call.func.attr == "fail":
        return True
    if call.func.attr != "append":
        return False
    target = call.func.value
    name = target.id if isinstance(target, ast.Name) else getattr(target, "attr", "")
    return "failure" in name


def enclosing_test(fn, call):
    """The condition that leads to `call`, or None if it is unconditional."""
    best = None
    for node in ast.walk(fn):
        if not isinstance(node, ast.If):
            continue
        for branch in (node.body, node.orelse):
            for stmt in branch:
                for inner in ast.walk(stmt):
                    if inner is call:
                        if best is None or node.lineno > best.lineno:
                            best = node
    return best.test if best is not None else None

def local_assignments(fn):
    """`name = <expr>` inside `fn`, latest assignment wins per line order."""
    out = {}
    for node in ast.walk(fn):
        if isinstance(node, ast.Assign) and len(node.targets) == 1:
            target = node.targets[0]
            if isinstance(target, ast.Name):
                out.setdefault(target.id, []).append(node.value)
    return out


def fields_of(expr, observed, assignments, seen=None):
    """Which observed fields `expr` reads directly.

    Local names are followed into what they were assigned, because the shape
    this rule is about hides the field behind exactly one: the assertion reads
    `told`, and `told` is a comprehension over `client.action_bar`.
    """
    if seen is None:
        seen = set()
    fields = attribute_names(expr) & observed
    for name in {n.id for n in ast.walk(expr) if isinstance(n, ast.Name)}:
        if name in seen or name not in assignments:
            continue
        seen.add(name)
        for value in assignments[name]:
            fields |= fields_of(value, observed, assignments, seen)
    return fields


def waited_before(fn, line, observed, waits):
    """Observed fields some wait earlier in `fn` has already covered."""
    covered = set()
    for call in ast.walk(fn):
        if not isinstance(call, ast.Call) or call.lineno >= line:
            continue
        func = call.func
        if not isinstance(func, ast.Attribute):
            continue
        if func.attr in WAITERS and call.args:
            covered |= attribute_names(call.args[0]) & observed
        elif (
            isinstance(func.value, ast.Name)
            and func.value.id == "self"
            and func.attr in waits
        ):
            # A helper that waits counts for its caller: `ask_podiums` polls
            # `client.podiums` itself, and the caller that reads the answer
            # never writes a wait of its own.
            covered |= waits[func.attr]
    return covered


def marker_reason(lines, line):
    """The escape hatch's reason, `None` if unmarked, `""` if left blank.

    Searched on the failing line and up through the comment block directly
    above it, so a reason too long for one line can be written as a paragraph
    like every other comment here rather than crammed onto one.
    """
    candidate = line
    while 1 <= candidate <= len(lines):
        found = MARKER.search(lines[candidate - 1])
        if found:
            return found.group("reason").strip()
        candidate -= 1
        if candidate < 1 or not lines[candidate - 1].lstrip().startswith("#"):
            break
    return None


def check(path):
    text = path.read_text()
    lines = text.splitlines()
    tree = ast.parse(text)
    observed = observed_fields(tree)
    if not observed:
        return []

    waits, methods = summarise(tree, observed)
    problems = []
    for name, fn in methods.items():
        # Only a function that waits at all. One that does not is reading a
        # fact somebody else established, and the bug this refuses is not
        # "forgot to wait", it is "waited, and for the wrong packet". Flagging
        # the rest is how a check becomes noise and the escape hatch becomes a
        # mute button.
        if not waits_of(fn, observed):
            continue
        assignments = local_assignments(fn)
        for call in ast.walk(fn):
            if not is_failure(call):
                continue
            test = enclosing_test(fn, call)
            subject = test if test is not None else ast.Tuple(elts=list(call.args))
            fields = fields_of(subject, observed, assignments)
            missing = fields - waited_before(fn, call.lineno, observed, waits)
            reason = marker_reason(lines, call.lineno)
            if reason == "":
                problems.append(
                    "%s:%d: `not-a-wire-assertion` with no reason after the colon. "
                    "The reason is the point: say which non-wire thing this asserts."
                    % (path.name, call.lineno)
                )
                continue
            if reason is not None:
                continue
            if missing:
                problems.append(
                    "%s:%d: in `%s`, this fails on %s, which the packet handler "
                    "fills, and no wait before it reads %s. Wait on the field you "
                    "assert on (`must_become`), or mark the line "
                    "`# not-a-wire-assertion: <why>`."
                    % (
                        path.name,
                        call.lineno,
                        name,
                        ", ".join("`%s`" % f for f in sorted(missing)),
                        "it" if len(missing) == 1 else "them",
                    )
                )
    return problems


def main():
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    tools = sorted((root / "tools").glob("*.py"))
    if not tools:
        print("no client scripts under %s/tools" % root)
        return 1

    problems = []
    for path in tools:
        try:
            problems.extend(check(path))
        except SyntaxError as error:
            problems.append("%s: does not parse: %s" % (path.name, error))

    for problem in problems:
        print(problem)
    print("")
    print("%d client scripts checked, %d problems" % (len(tools), len(problems)))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
