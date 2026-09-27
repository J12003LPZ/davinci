"""Bounded syntax facts only. Never imports or executes inspected source."""
import ast
import json
import sys


class Unsupported(Exception):
    pass


class Inspector:
    def __init__(self):
        self.values = {}
        self.nonempty = set()
        self.symbols = set()
        self.checked = set()
        self.aliases = {}
        self.functions = {}
        self.returns = set()
        self.unit_checks = set()
        self.unit_classes = set()
        self.in_test = False
        self.in_function = False

    def name(self, node):
        if isinstance(node, ast.Name):
            name = self.aliases.get(node.id, node.id)
        elif isinstance(node, ast.Attribute):
            name = self.name(node.value) + "." + node.attr
        else:
            return ""
        return name.removeprefix("builtins.")

    def symbol(self, node):
        return (isinstance(node, ast.Name) and node.id in self.symbols
                or isinstance(node, ast.Attribute) and self.symbol(node.value))

    def dependencies(self, node):
        if node is None or isinstance(node, ast.Constant):
            return set()
        if isinstance(node, ast.Name):
            return self.values.get(node.id, set()).copy()
        if isinstance(node, (ast.Lambda, ast.NamedExpr, ast.Await, ast.Yield,
                             ast.ListComp, ast.SetComp, ast.DictComp, ast.GeneratorExp)):
            raise Unsupported()
        if isinstance(node, ast.BoolOp):
            # Only unconditional operands establish association. Reject obvious
            # constant short-circuit checks instead of crediting unreachable code.
            if isinstance(node.op, ast.Or) or any(isinstance(value, ast.Constant) for value in node.values):
                raise Unsupported()
        if isinstance(node, ast.Call):
            name = self.name(node.func)
            if name in {"exec", "eval", "compile", "__import__", "getattr", "setattr",
                        "sys.exit", "os._exit", "exit", "quit", "self.skipTest"} or any(
                            name.startswith(terminal + ".") for terminal in {"sys.exit", "os._exit", "exit", "quit"}):
                raise Unsupported()
            # Existence of an imported symbol is not a behavioral check.
            if name in {"hasattr", "callable", "id", "type", "all", "any"}:
                return set()
            if name in {"bool", "str", "repr", "ascii", "format", "len", "isinstance", "issubclass"} and any(
                    self.symbol(value) for value in node.args):
                return set()
            if name == "isinstance" and len(node.args) >= 2 and self.name(node.args[1]) == "object":
                return set()
            if name in {"len", "bool", "tuple", "list", "set", "dict"} and node.args and self.nonempty_literal(node.args[0]):
                return set()
            if name in self.functions:
                returned, checked = self.functions[name]
                self.checked.update(checked)
                return returned.copy()
            if name == "subprocess.run" and node.args and isinstance(node.args[0], (ast.List, ast.Tuple)):
                argv = node.args[0].elts
                if (len(argv) >= 2 and self.name(argv[0]) == "sys.executable"
                        and isinstance(argv[1], ast.Constant) and isinstance(argv[1].value, str)
                        and not argv[1].value.startswith("-")):
                    return {("file", argv[1].value)}
            if name in {"pathlib.Path", "open"} and node.args:
                value = node.args[0]
                if isinstance(value, ast.Constant) and isinstance(value.value, str):
                    return {("path", value.value)}
        result = set()
        for child in ast.iter_child_nodes(node):
            result.update(self.dependencies(child))
        if isinstance(node, ast.Call) and (self.name(node.func) == "json.load"
                or isinstance(node.func, ast.Attribute) and node.func.attr in {
                    "read", "read_text", "read_bytes", "exists", "is_file", "is_dir"}):
            result = {("file" if kind == "path" else kind, value) for kind, value in result}
        return result

    def predicate(self, node):
        """Only checks that can fail on an observed value establish coverage."""
        if isinstance(node, (ast.Constant, ast.Tuple, ast.List, ast.Set, ast.Dict, ast.Attribute)):
            return set()
        if isinstance(node, ast.Name) and (node.id in self.symbols or node.id in self.nonempty):
            return set()
        if isinstance(node, ast.BoolOp):
            if not isinstance(node.op, ast.And):
                raise Unsupported()
            return set().union(*(self.predicate(value) for value in node.values))
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Not):
            return self.predicate(node.operand)
        if isinstance(node, ast.Compare):
            operands = [node.left] + node.comparators
            containers = (ast.Tuple, ast.List, ast.Set, ast.Dict)
            for left, op, right in zip(operands, node.ops, operands[1:]):
                # Empty/nonempty container shape is independent of the values
                # computed inside it, including a prior container assignment.
                left_empty = isinstance(left, containers) and not self.nonempty_literal(left)
                right_empty = isinstance(right, containers) and not self.nonempty_literal(right)
                if ((isinstance(left, containers) or self.nonempty_literal(left))
                        and (isinstance(right, containers) or self.nonempty_literal(right))):
                    return set()
                if ((left_empty and self.nonempty_literal(right))
                        or (right_empty and self.nonempty_literal(left))
                        or (isinstance(op, (ast.In, ast.NotIn)) and right_empty)
                        or (isinstance(op, (ast.Is, ast.IsNot)) and isinstance(right, containers))):
                    return set()
                if isinstance(left, (ast.Tuple, ast.List)) and isinstance(right, type(left)):
                    if len(left.elts) != len(right.elts) and isinstance(op, (ast.Eq, ast.NotEq)):
                        return set()
            if any(ast.dump(left) == ast.dump(right) for left, right in zip(operands, operands[1:])):
                return set()
            if any(isinstance(op, (ast.In, ast.NotIn)) and isinstance(right, (ast.List, ast.Tuple, ast.Set))
                   and any(ast.dump(left) == ast.dump(item) for item in right.elts)
                   for left, op, right in zip(operands, node.ops, operands[1:])):
                return set()
            if any(isinstance(op, (ast.Is, ast.IsNot)) for op in node.ops) and any(self.symbol(value) for value in operands):
                return set()
            if any(isinstance(op, ast.NotEq) for op in node.ops) and any(self.symbol(value) for value in operands) and any(
                    isinstance(value, ast.Constant) and value.value is None for value in operands):
                return set()
        return self.dependencies(node)

    def bind(self, target, dependencies):
        if isinstance(target, ast.Name):
            if target.id in self.unit_classes:
                raise Unsupported()
            if (self.functions or self.unit_checks) and self.values.get(target.id):
                # Deferred bodies must not retain facts about a rebound global.
                raise Unsupported()
            self.values[target.id] = dependencies.copy()
            self.aliases.pop(target.id, None)
            self.functions.pop(target.id, None)
            self.nonempty.discard(target.id)
            self.symbols.discard(target.id)
        elif isinstance(target, (ast.Tuple, ast.List)):
            for item in target.elts:
                self.bind(item, dependencies)
        else:
            raise Unsupported()

    def nonempty_literal(self, node):
        if isinstance(node, (ast.List, ast.Tuple, ast.Set)):
            return bool(node.elts)
        if isinstance(node, ast.Dict):
            return bool(node.keys)
        if isinstance(node, ast.Name):
            return node.id in self.nonempty
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            return (node.func.attr in {"items", "values", "keys"}
                    and not node.args and not node.keywords
                    and self.nonempty_literal(node.func.value))
        return False

    def failure(self, node):
        if not isinstance(node, ast.Raise):
            return False
        if isinstance(node.exc, ast.Name):
            return self.name(node.exc) == "AssertionError"
        if not isinstance(node.exc, ast.Call):
            return False
        name = self.name(node.exc.func)
        if name == "AssertionError":
            return True
        return (name == "SystemExit" and len(node.exc.args) == 1
                and isinstance(node.exc.args[0], ast.Constant)
                and isinstance(node.exc.args[0].value, int) and node.exc.args[0].value != 0)

    def expected_exception(self, node, following):
        if node.finalbody or len(node.handlers) != 1:
            raise Unsupported()
        handler = node.handlers[0]
        if (not isinstance(handler.type, ast.Name)
                or handler.type.id in {"Exception", "BaseException", "AssertionError"}):
            raise Unsupported()
        # Assertions in the try body could be swallowed, so only actual calls
        # are accepted here. The exception's concrete type is part of the check.
        if not node.body or any(not isinstance(item, ast.Expr)
                                or not isinstance(item.value, ast.Call) for item in node.body):
            raise Unsupported()
        deps = set().union(*(self.dependencies(item.value) for item in node.body))
        if not deps:
            raise Unsupported()
        terminal_else = len(node.orelse) == 1 and self.failure(node.orelse[0])
        continues = (not node.orelse and len(handler.body) == 1
                     and isinstance(handler.body[0], ast.Continue)
                     and following is not None and self.failure(following))
        if not terminal_else and not continues:
            raise Unsupported()
        if handler.name:
            self.values[handler.name] = deps
        for item in handler.body:
            if isinstance(item, ast.Assert):
                self.dependencies(item.test)
            elif not isinstance(item, (ast.Pass, ast.Continue)):
                raise Unsupported()
        self.checked.update(deps)
        return continues

    def statements(self, body):
        skip = False
        for index, node in enumerate(body):
            if skip:
                skip = False
                continue
            if isinstance(node, ast.Import):
                for alias in node.names:
                    name = alias.asname or alias.name.split(".")[0]
                    if name in self.unit_classes:
                        raise Unsupported()
                    self.values[name] = {("module", alias.name)}
                    self.aliases[name] = alias.name
                    self.symbols.add(name)
            elif isinstance(node, ast.ImportFrom):
                if node.level or not node.module or any(a.name == "*" for a in node.names):
                    raise Unsupported()
                for alias in node.names:
                    name = alias.asname or alias.name
                    if name in self.unit_classes:
                        raise Unsupported()
                    self.values[name] = {("module", node.module), ("module", node.module + "." + alias.name)}
                    self.aliases[name] = node.module + "." + alias.name
                    self.symbols.add(name)
            elif isinstance(node, ast.Assign):
                deps = self.dependencies(node.value)
                nonempty = self.nonempty_literal(node.value)
                symbol = self.symbol(node.value)
                alias = self.name(node.value) if isinstance(node.value, (ast.Name, ast.Attribute)) else ""
                for target in node.targets:
                    self.bind(target, deps)
                    if nonempty and isinstance(target, ast.Name):
                        self.nonempty.add(target.id)
                    if symbol and isinstance(target, ast.Name):
                        self.symbols.add(target.id)
                    if alias and isinstance(target, ast.Name):
                        self.aliases[target.id] = alias
            elif isinstance(node, ast.Assert):
                self.checked.update(self.predicate(node.test))
            elif isinstance(node, ast.Expr):
                deps = self.dependencies(node.value)
                if isinstance(node.value, ast.Call):
                    name = self.name(node.value.func)
                    if self.in_test and name in {"self.assertEqual", "self.assertTrue", "self.assertFalse", "self.assertIn", "self.assertNotIn"}:
                        if name in {"self.assertTrue", "self.assertFalse"}:
                            if node.value.args:
                                self.checked.update(self.predicate(node.value.args[0]))
                        elif len(node.value.args) >= 2:
                            operator = {"self.assertEqual": ast.Eq, "self.assertIn": ast.In,
                                        "self.assertNotIn": ast.NotIn}[name]
                            self.checked.update(self.predicate(ast.Compare(
                                left=node.value.args[0], ops=[operator()], comparators=[node.value.args[1]])))
                    if name == "unittest.main":
                        if node.value.args or node.value.keywords or index + 1 != len(body):
                            raise Unsupported()
                        self.checked.update(self.unit_checks)
            elif isinstance(node, ast.For):
                if node.orelse or not self.nonempty_literal(node.iter):
                    raise Unsupported()
                self.bind(node.target, self.dependencies(node.iter))
                self.statements(node.body)
            elif isinstance(node, ast.Try):
                if node.finalbody and not node.handlers and not node.orelse:
                    # Literal unlink cleanup cannot swallow an earlier failure.
                    if any(not isinstance(item, ast.Expr) or not isinstance(item.value, ast.Call)
                           or not isinstance(item.value.func, ast.Attribute)
                           or item.value.func.attr != "unlink" for item in node.finalbody):
                        raise Unsupported()
                    self.statements(node.body)
                    continue
                following = body[index + 1] if index + 1 < len(body) else None
                skip = self.expected_exception(node, following)
            elif isinstance(node, ast.If):
                if node.orelse or len(node.body) != 1 or not self.failure(node.body[0]):
                    raise Unsupported()
                self.checked.update(self.predicate(node.test))
            elif isinstance(node, ast.With):
                # These standard-library managers do not suppress exceptions.
                # No user-defined or contextlib.suppress manager is accepted.
                for item in node.items:
                    expr = item.context_expr
                    if (self.in_test and isinstance(expr, ast.Call)
                            and self.name(expr.func) in {"self.assertRaises", "self.assertRaisesRegex"}
                            and expr.args and isinstance(expr.args[0], ast.Name)
                            and expr.args[0].id not in {"Exception", "BaseException", "AssertionError"}):
                        if any(not isinstance(stmt, ast.Expr) or not isinstance(stmt.value, ast.Call) for stmt in node.body):
                            raise Unsupported()
                        self.checked.update(set().union(*(self.dependencies(stmt.value) for stmt in node.body)))
                        continue
                    if not isinstance(expr, ast.Call) or self.name(expr.func) not in {
                        "tempfile.TemporaryDirectory", "contextlib.redirect_stdout",
                        "contextlib.redirect_stderr", "tempfile.NamedTemporaryFile",
                    }:
                        raise Unsupported()
                    deps = self.dependencies(expr)
                    if item.optional_vars:
                        self.bind(item.optional_vars, deps)
                self.statements(node.body)
            elif isinstance(node, ast.FunctionDef):
                if (node.decorator_list or node.args.defaults or node.args.kw_defaults
                        or node.name == "load_tests" or node.name in self.unit_classes):
                    raise Unsupported()
                child = Inspector()
                child.values = {key: value.copy() for key, value in self.values.items()}
                child.aliases = self.aliases.copy()
                child.symbols = self.symbols.copy()
                child.nonempty = self.nonempty.copy()
                child.in_test = self.in_test
                child.in_function = True
                for arg in node.args.posonlyargs + node.args.args + node.args.kwonlyargs:
                    child.bind(ast.Name(id=arg.arg), set())
                if node.args.vararg:
                    child.bind(ast.Name(id=node.args.vararg.arg), set())
                if node.args.kwarg:
                    child.bind(ast.Name(id=node.args.kwarg.arg), set())
                child.statements(node.body)
                self.functions[node.name] = (child.returns, child.checked)
                if self.in_test and node.name.startswith("test"):
                    self.unit_checks.update(child.checked)
            elif isinstance(node, ast.Return):
                self.returns.update(self.dependencies(node.value))
                if index + 1 != len(body):
                    raise Unsupported()
            elif isinstance(node, ast.ClassDef):
                if (node.decorator_list or node.keywords or len(node.bases) != 1
                        or self.name(node.bases[0]) != "unittest.TestCase"
                        or self.in_function
                        or node.name in self.unit_classes
                        or any(not isinstance(item, ast.FunctionDef) or not item.name.startswith("test") for item in node.body)
                        or len({item.name for item in node.body}) != len(node.body)):
                    raise Unsupported()
                self.unit_classes.add(node.name)
                self.in_test = True
                self.statements(node.body)
                self.in_test = False
            elif not isinstance(node, ast.Pass):
                raise Unsupported()


def inspect_source(source):
    empty = {"checked_modules": [], "checked_files": [], "imports": [], "reason": "unsupported_syntax"}
    if not isinstance(source, str) or len(source.encode("utf-8")) > 65536:
        return empty
    try:
        tree = ast.parse(source)
        if sum(1 for _ in ast.walk(tree)) > 8192:
            return empty
        # Only unconditional module-level imports establish a local dependency.
        empty["imports"] = sorted({name for node in tree.body for name in
            ([alias.name for alias in node.names] if isinstance(node, ast.Import) else
             [node.module] + [node.module + "." + alias.name for alias in node.names]
             if isinstance(node, ast.ImportFrom) and node.level == 0 and node.module else [])})
        inspector = Inspector()
        inspector.statements(tree.body)
        return {
            "checked_modules": sorted(value for kind, value in inspector.checked if kind == "module"),
            "checked_files": sorted(value for kind, value in inspector.checked if kind == "file"),
            "imports": empty["imports"],
            "reason": "recognized" if inspector.checked else "no_applicable_check",
        }
    except (SyntaxError, ValueError, TypeError, RecursionError, Unsupported):
        return empty


if __name__ == "__main__":
    try:
        request = json.loads(sys.stdin.buffer.read(262145))
        result = inspect_source(request.get("source"))
    except (ValueError, AttributeError):
        result = inspect_source(None)
    encoded = json.dumps(result)
    print(encoded if len(encoded.encode("utf-8")) <= 16384 else json.dumps(inspect_source(None)))
