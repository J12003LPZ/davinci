"""Write the benchmark tasks: repo/ (what the agent sees), hidden/ (grading
tests), solution/ (reference files used only to validate the hidden tests)."""
import json
import os
import shutil
import textwrap

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "tasks")


def w(base, rel, text):
    path = os.path.join(base, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(textwrap.dedent(text).lstrip("\n"))


def task(tid, prompt, allowed, repo, hidden, solution):
    base = os.path.join(ROOT, tid)
    shutil.rmtree(base, ignore_errors=True)
    for rel, text in repo.items():
        w(os.path.join(base, "repo"), rel, text)
    for rel, text in hidden.items():
        w(os.path.join(base, "hidden"), rel, text)
    for rel, text in solution.items():
        w(os.path.join(base, "solution"), rel, text)
    with open(os.path.join(base, "task.json"), "w", encoding="utf-8") as f:
        json.dump({"id": tid, "prompt": prompt, "allowed": allowed}, f, indent=2)


# ---------------------------------------------------------------- T1
task(
    "t1-intervals",
    "merge_intervals in intervals.py gives wrong results for some inputs: intervals that "
    "touch (like [1, 2] and [2, 3]) should be merged, input may be unsorted, and the caller's "
    "list must not be modified. Fix it.",
    ["intervals.py", "test_intervals.py"],
    {
        "README.md": "# intervals\n\nUtilities for closed integer intervals.\n",
        "intervals.py": '''
            def merge_intervals(intervals):
                """Merge overlapping closed intervals given as [start, end] lists.

                Returns a new list of [start, end] lists sorted by start.
                """
                result = []
                for interval in intervals:
                    if result and interval[0] < result[-1][1]:
                        result[-1][1] = max(result[-1][1], interval[1])
                    else:
                        result.append(interval)
                return result
        ''',
        "test_intervals.py": '''
            from intervals import merge_intervals


            def test_basic_overlap():
                assert merge_intervals([[1, 3], [2, 5]]) == [[1, 5]]
        ''',
    },
    {
        "test_hidden_intervals.py": '''
            import copy
            from intervals import merge_intervals


            def test_overlap():
                assert merge_intervals([[1, 3], [2, 5], [7, 8]]) == [[1, 5], [7, 8]]


            def test_touching():
                assert merge_intervals([[1, 2], [2, 3]]) == [[1, 3]]


            def test_unsorted():
                assert merge_intervals([[5, 6], [1, 2], [2, 4]]) == [[1, 4], [5, 6]]


            def test_contained():
                assert merge_intervals([[1, 10], [2, 3], [4, 5]]) == [[1, 10]]


            def test_empty_and_single():
                assert merge_intervals([]) == []
                assert merge_intervals([[3, 3]]) == [[3, 3]]


            def test_input_not_mutated():
                data = [[4, 6], [1, 5]]
                before = copy.deepcopy(data)
                merge_intervals(data)
                assert data == before


            def test_gap_of_one_not_merged():
                assert merge_intervals([[1, 2], [3, 4]]) == [[1, 2], [3, 4]]
        ''',
    },
    {
        "intervals.py": '''
            def merge_intervals(intervals):
                result = []
                for start, end in sorted(([s, e] for s, e in intervals), key=lambda i: i[0]):
                    if result and start <= result[-1][1]:
                        result[-1][1] = max(result[-1][1], end)
                    else:
                        result.append([start, end])
                return result
        ''',
    },
)

# ---------------------------------------------------------------- T2
task(
    "t2-duration",
    "Implement parse_duration in durations.py. It takes strings like \"1h30m\", \"2d\", "
    "\"90s\", \"1.5h\" or \"1h 15m 10s\" (units d, h, m, s; each unit at most once, in any "
    "order; spaces allowed between parts; numbers may be decimals) and returns the total number "
    "of seconds as a float. Anything else (empty string, unknown unit, missing number, repeated "
    "unit, negative numbers) must raise ValueError.",
    ["durations.py", "test_durations.py"],
    {
        "durations.py": '''
            def parse_duration(text):
                """Parse a duration like "1h30m" into seconds (float)."""
                raise NotImplementedError
        ''',
    },
    {
        "test_hidden_durations.py": '''
            import pytest
            from durations import parse_duration


            @pytest.mark.parametrize("text,expected", [
                ("90s", 90.0),
                ("1h30m", 5400.0),
                ("2d", 172800.0),
                ("1.5h", 5400.0),
                ("1h 15m 10s", 4510.0),
                ("10s1m", 70.0),
                ("0s", 0.0),
                ("  3m  ", 180.0),
            ])
            def test_valid(text, expected):
                assert parse_duration(text) == pytest.approx(expected)


            @pytest.mark.parametrize("text", ["", "   ", "10x", "h", "1h1h", "-5s", "5", "1h-30m", "abc"])
            def test_invalid(text):
                with pytest.raises(ValueError):
                    parse_duration(text)
        ''',
    },
    {
        "durations.py": '''
            import re

            _UNITS = {"d": 86400, "h": 3600, "m": 60, "s": 1}
            _PART = re.compile(r"\\s*(\\d+(?:\\.\\d+)?)([dhms])\\s*")


            def parse_duration(text):
                if not isinstance(text, str) or not text.strip():
                    raise ValueError("empty duration")
                pos, seen, total = 0, set(), 0.0
                while pos < len(text):
                    match = _PART.match(text, pos)
                    if not match:
                        raise ValueError(f"bad duration: {text!r}")
                    unit = match.group(2)
                    if unit in seen:
                        raise ValueError("repeated unit")
                    seen.add(unit)
                    total += float(match.group(1)) * _UNITS[unit]
                    pos = match.end()
                return total
        ''',
    },
)

# ---------------------------------------------------------------- T3
task(
    "t3-lru",
    "Implement the LRUCache class in lru.py: LRUCache(capacity) with get(key, default=None) and "
    "put(key, value). get and put both mark the key as most recently used; when a put would "
    "exceed capacity, evict the least recently used key. len(cache) gives the number of items "
    "and `key in cache` checks membership WITHOUT changing recency. A capacity below 1 raises "
    "ValueError. The existing session store uses this class.",
    ["lru.py", "test_lru.py"],
    {
        "lru.py": '''
            class LRUCache:
                """Least-recently-used cache. See the task description."""

                def __init__(self, capacity):
                    raise NotImplementedError
        ''',
        "sessions.py": '''
            from lru import LRUCache


            class SessionStore:
                def __init__(self, capacity=2):
                    self._cache = LRUCache(capacity)

                def login(self, user, token):
                    self._cache.put(token, user)

                def user_for(self, token):
                    return self._cache.get(token)
        ''',
    },
    {
        "test_hidden_lru.py": '''
            import pytest
            from lru import LRUCache
            from sessions import SessionStore


            def test_evicts_least_recent():
                c = LRUCache(2)
                c.put("a", 1)
                c.put("b", 2)
                assert c.get("a") == 1
                c.put("c", 3)
                assert "b" not in c
                assert c.get("a") == 1 and c.get("c") == 3


            def test_put_updates_recency_and_value():
                c = LRUCache(2)
                c.put("a", 1)
                c.put("b", 2)
                c.put("a", 10)
                c.put("c", 3)
                assert c.get("a") == 10
                assert "b" not in c
                assert len(c) == 2


            def test_contains_does_not_touch_recency():
                c = LRUCache(2)
                c.put("a", 1)
                c.put("b", 2)
                assert "a" in c
                c.put("c", 3)
                assert "a" not in c


            def test_default_and_len():
                c = LRUCache(1)
                assert c.get("missing") is None
                assert c.get("missing", 5) == 5
                assert len(c) == 0
                c.put("x", None)
                assert "x" in c and c.get("x", 7) is None


            def test_bad_capacity():
                with pytest.raises(ValueError):
                    LRUCache(0)


            def test_sessions():
                s = SessionStore(capacity=2)
                s.login("ann", "t1")
                s.login("bob", "t2")
                s.user_for("t1")
                s.login("cy", "t3")
                assert s.user_for("t2") is None
                assert s.user_for("t1") == "ann"
        ''',
    },
    {
        "lru.py": '''
            from collections import OrderedDict


            class LRUCache:
                def __init__(self, capacity):
                    if capacity < 1:
                        raise ValueError("capacity must be >= 1")
                    self.capacity = capacity
                    self._data = OrderedDict()

                def get(self, key, default=None):
                    if key not in self._data:
                        return default
                    self._data.move_to_end(key)
                    return self._data[key]

                def put(self, key, value):
                    self._data[key] = value
                    self._data.move_to_end(key)
                    while len(self._data) > self.capacity:
                        self._data.popitem(last=False)

                def __len__(self):
                    return len(self._data)

                def __contains__(self, key):
                    return key in self._data
        ''',
    },
)

# ---------------------------------------------------------------- T4
task(
    "t4-rename",
    "Rename calc_total in shop/pricing.py to compute_total everywhere in the project (remove the "
    "old name completely), and give it a new optional parameter discount: a percentage from 0 to "
    "100 (default 0) applied to the subtotal before tax. A discount outside 0..100 raises "
    "ValueError. Existing behavior without a discount must not change.",
    ["shop/pricing.py", "shop/orders.py", "shop/report.py", "cli.py", "tests/test_pricing.py"],
    {
        "shop/__init__.py": "",
        "shop/pricing.py": '''
            TAX_RATE = 0.08


            def calc_total(items):
                """items: list of (unit_price, quantity). Returns total with tax, rounded to cents."""
                subtotal = sum(price * qty for price, qty in items)
                return round(subtotal * (1 + TAX_RATE), 2)
        ''',
        "shop/orders.py": '''
            from shop.pricing import calc_total


            class Order:
                def __init__(self):
                    self.items = []

                def add(self, price, qty=1):
                    self.items.append((price, qty))

                def total(self):
                    return calc_total(self.items)
        ''',
        "shop/report.py": '''
            from shop import pricing


            def daily_revenue(orders):
                return round(sum(pricing.calc_total(o.items) for o in orders), 2)
        ''',
        "cli.py": '''
            import sys

            from shop.pricing import calc_total


            def main(argv):
                items = []
                for arg in argv:
                    price, qty = arg.split("x")
                    items.append((float(price), int(qty)))
                print(f"{calc_total(items):.2f}")


            if __name__ == "__main__":
                main(sys.argv[1:])
        ''',
        "tests/test_pricing.py": '''
            from shop.pricing import calc_total


            def test_total():
                assert calc_total([(10.0, 2)]) == 21.6
        ''',
    },
    {
        "test_hidden_rename.py": '''
            import pathlib
            import pytest
            from shop import pricing
            from shop.orders import Order
            from shop.report import daily_revenue
            import cli


            def test_new_name_and_old_gone():
                assert pricing.compute_total([(10.0, 2)]) == 21.6
                assert not hasattr(pricing, "calc_total")
                root = pathlib.Path(__file__).parent
                for path in root.rglob("*.py"):
                    if "hidden" in path.name:
                        continue
                    assert "calc_total" not in path.read_text(encoding="utf-8"), path


            def test_discount():
                assert pricing.compute_total([(100.0, 1)], discount=10) == 97.2
                assert pricing.compute_total([(100.0, 1)], discount=0) == 108.0
                assert pricing.compute_total([(100.0, 1)], discount=100) == 0.0


            @pytest.mark.parametrize("bad", [-1, 101, 150])
            def test_bad_discount(bad):
                with pytest.raises(ValueError):
                    pricing.compute_total([(1.0, 1)], discount=bad)


            def test_callers(capsys):
                o = Order()
                o.add(10.0, 2)
                assert o.total() == 21.6
                assert daily_revenue([o, o]) == 43.2
                cli.main(["10x2"])
                assert capsys.readouterr().out.strip() == "21.60"
        ''',
    },
    {
        "shop/pricing.py": '''
            TAX_RATE = 0.08


            def compute_total(items, discount=0):
                if not 0 <= discount <= 100:
                    raise ValueError("discount must be between 0 and 100")
                subtotal = sum(price * qty for price, qty in items)
                subtotal *= 1 - discount / 100
                return round(subtotal * (1 + TAX_RATE), 2)
        ''',
        "shop/orders.py": '''
            from shop.pricing import compute_total


            class Order:
                def __init__(self):
                    self.items = []

                def add(self, price, qty=1):
                    self.items.append((price, qty))

                def total(self):
                    return compute_total(self.items)
        ''',
        "shop/report.py": '''
            from shop import pricing


            def daily_revenue(orders):
                return round(sum(pricing.compute_total(o.items) for o in orders), 2)
        ''',
        "cli.py": '''
            import sys

            from shop.pricing import compute_total


            def main(argv):
                items = []
                for arg in argv:
                    price, qty = arg.split("x")
                    items.append((float(price), int(qty)))
                print(f"{compute_total(items):.2f}")


            if __name__ == "__main__":
                main(sys.argv[1:])
        ''',
        "tests/test_pricing.py": '''
            from shop.pricing import compute_total


            def test_total():
                assert compute_total([(10.0, 2)]) == 21.6
        ''',
    },
)

# ---------------------------------------------------------------- T5
task(
    "t5-csv",
    "The regional sales totals from report.py are wrong for the files our finance team exports "
    "(see data/export.csv): some regions show up twice and some amounts are off. Find the causes "
    "and fix region_totals so it handles these exports correctly.",
    ["report.py", "test_report.py"],
    {
        "report.py": '''
            import csv


            def region_totals(path):
                """Return {region: total_amount} for a sales CSV with columns region,amount."""
                totals = {}
                with open(path, newline="") as f:
                    for row in csv.DictReader(f):
                        region = row["region"]
                        totals[region] = totals.get(region, 0.0) + float(row["amount"].replace(",", ""))
                return totals
        ''',
        "data/export.csv": '﻿region,amount\nNorth,"1,200.50"\nnorth ,300\nSouth,$45.25\n  South,"2,000"\nEast,10\n',
    },
    {
        "test_hidden_report.py": '''
            import pytest
            from report import region_totals


            def write(tmp_path, text):
                p = tmp_path / "s.csv"
                p.write_bytes(text.encode("utf-8"))
                return p


            def test_export_file():
                t = region_totals("data/export.csv")
                assert t == pytest.approx({"North": 1500.5, "South": 2045.25, "East": 10.0})


            def test_bom_and_whitespace(tmp_path):
                p = write(tmp_path, "\\ufeffregion,amount\\n West ,5\\nWest,5\\n")
                assert region_totals(p) == pytest.approx({"West": 10.0})


            def test_plain_file(tmp_path):
                p = write(tmp_path, "region,amount\\nA,1.5\\nB,2\\nA,3\\n")
                assert region_totals(p) == pytest.approx({"A": 4.5, "B": 2.0})


            def test_currency_and_thousands(tmp_path):
                p = write(tmp_path, 'region,amount\\nA,"$1,000.25"\\nA,$0.75\\n')
                assert region_totals(p) == pytest.approx({"A": 1001.0})
        ''',
    },
    {
        "report.py": '''
            import csv


            def region_totals(path):
                totals = {}
                names = {}
                with open(path, newline="", encoding="utf-8-sig") as f:
                    for row in csv.DictReader(f):
                        raw = row["region"].strip()
                        key = raw.lower()
                        names.setdefault(key, raw[:1].upper() + raw[1:].lower())
                        amount = float(row["amount"].strip().replace("$", "").replace(",", ""))
                        totals[key] = totals.get(key, 0.0) + amount
                return {names[k]: v for k, v in totals.items()}
        ''',
    },
)

# ---------------------------------------------------------------- T6
task(
    "t6-bookings",
    "Room bookings that end on the same day another booking starts are rejected as overlapping, "
    "but check-out day is free for the next guest. Fix it without letting real overlaps through.",
    ["bookings.py", "test_bookings.py"],
    {
        "bookings.py": '''
            from datetime import date


            class Calendar:
                """Bookings are [start, end) date ranges: end is the check-out day."""

                def __init__(self):
                    self.bookings = []

                def overlaps(self, start, end):
                    for s, e in self.bookings:
                        if start <= e and s <= end:
                            return True
                    return False

                def book(self, start, end):
                    if end <= start:
                        raise ValueError("end must be after start")
                    if self.overlaps(start, end):
                        raise ValueError("overlapping booking")
                    self.bookings.append((start, end))
        ''',
    },
    {
        "test_hidden_bookings.py": '''
            from datetime import date
            import pytest
            from bookings import Calendar

            D = lambda d: date(2026, 5, d)


            def test_back_to_back_allowed():
                c = Calendar()
                c.book(D(1), D(3))
                c.book(D(3), D(5))
                c.book(D(0 + 5), D(6))


            def test_before_allowed():
                c = Calendar()
                c.book(D(5), D(8))
                c.book(D(2), D(5))


            @pytest.mark.parametrize("s,e", [(2, 4), (1, 3), (0 + 2, 3), (1, 10), (2, 3)])
            def test_real_overlaps_rejected(s, e):
                c = Calendar()
                c.book(D(1), D(4))
                with pytest.raises(ValueError):
                    c.book(D(s), D(e))


            def test_zero_length_rejected():
                with pytest.raises(ValueError):
                    Calendar().book(D(2), D(2))
        ''',
    },
    {
        "bookings.py": '''
            from datetime import date


            class Calendar:
                def __init__(self):
                    self.bookings = []

                def overlaps(self, start, end):
                    for s, e in self.bookings:
                        if start < e and s < end:
                            return True
                    return False

                def book(self, start, end):
                    if end <= start:
                        raise ValueError("end must be after start")
                    if self.overlaps(start, end):
                        raise ValueError("overlapping booking")
                    self.bookings.append((start, end))
        ''',
    },
)

# ---------------------------------------------------------------- T7
task(
    "t7-cli",
    "Add a --top N option to wordcount.py that prints the N most frequent words, one per line "
    "as \"word count\", ordered by count (highest first) and then alphabetically. Words are "
    "case-insensitive and punctuation is ignored (apostrophes inside words like don't are kept). "
    "Without --top the output must stay exactly as it is today. N must be a positive integer.",
    ["wordcount.py", "test_wordcount.py"],
    {
        "wordcount.py": '''
            import argparse
            import sys


            def main(argv=None):
                parser = argparse.ArgumentParser(description="Count words in a file")
                parser.add_argument("path")
                args = parser.parse_args(argv)
                with open(args.path, encoding="utf-8") as f:
                    text = f.read()
                print(len(text.split()))
                return 0


            if __name__ == "__main__":
                sys.exit(main())
        ''',
    },
    {
        "test_hidden_wordcount.py": '''
            import pytest
            import wordcount


            def run(tmp_path, capsys, text, *extra):
                p = tmp_path / "in.txt"
                p.write_text(text, encoding="utf-8")
                wordcount.main([str(p), *extra])
                return capsys.readouterr().out


            def test_default_unchanged(tmp_path, capsys):
                assert run(tmp_path, capsys, "a b  c\\nd") == "4\\n"


            def test_top(tmp_path, capsys):
                out = run(tmp_path, capsys, "The cat. the Dog! the cat, don't DON'T bird", "--top", "3")
                assert out.splitlines() == ["the 3", "cat 2", "don't 2"]


            def test_top_larger_than_vocab(tmp_path, capsys):
                out = run(tmp_path, capsys, "b a", "--top", "5")
                assert out.splitlines() == ["a 1", "b 1"]


            @pytest.mark.parametrize("bad", ["0", "-2", "x"])
            def test_bad_n(tmp_path, capsys, bad):
                with pytest.raises(SystemExit) as info:
                    run(tmp_path, capsys, "a", "--top", bad)
                assert info.value.code != 0
        ''',
    },
    {
        "wordcount.py": '''
            import argparse
            import re
            import sys
            from collections import Counter


            def positive(text):
                value = int(text)
                if value < 1:
                    raise argparse.ArgumentTypeError("must be positive")
                return value


            def main(argv=None):
                parser = argparse.ArgumentParser(description="Count words in a file")
                parser.add_argument("path")
                parser.add_argument("--top", type=positive)
                args = parser.parse_args(argv)
                with open(args.path, encoding="utf-8") as f:
                    text = f.read()
                if args.top is None:
                    print(len(text.split()))
                    return 0
                words = re.findall(r"[a-z0-9]+(?:'[a-z0-9]+)*", text.lower())
                ranked = sorted(Counter(words).items(), key=lambda kv: (-kv[1], kv[0]))
                for word, count in ranked[: args.top]:
                    print(f"{word} {count}")
                return 0


            if __name__ == "__main__":
                sys.exit(main())
        ''',
    },
)

# ---------------------------------------------------------------- T8
_T8_TESTS = '''
    import pytest
    from calc import evaluate


    @pytest.mark.parametrize("expr,value", [
        ("1 + 2", 3),
        ("2 * 3 + 4", 10),
        ("2 + 3 * 4", 14),
        ("10 - 3 - 2", 5),
        ("16 / 4 / 2", 2),
        ("(1 + 2) * 3", 9),
    ])
    def test_evaluate(expr, value):
        assert evaluate(expr) == value
'''
task(
    "t8-calc",
    "The test suite for calc.py fails. Fix calc.py (do not change the tests) so arithmetic "
    "follows normal precedence and left-to-right associativity. Unary minus (like -3 or "
    "-(2+1)) should also work.",
    ["calc.py"],
    {
        "calc.py": '''
            import re

            TOKEN = re.compile(r"\\s*(\\d+(?:\\.\\d+)?|[-+*/()])")


            def tokenize(text):
                pos, tokens = 0, []
                text = text.rstrip()
                while pos < len(text):
                    match = TOKEN.match(text, pos)
                    if not match:
                        raise ValueError(f"bad input at {pos}")
                    tokens.append(match.group(1))
                    pos = match.end()
                return tokens


            def evaluate(text):
                tokens = tokenize(text)
                value, rest = _expr(tokens)
                if rest:
                    raise ValueError("trailing input")
                return value


            def _expr(tokens):
                left, rest = _term(tokens)
                if rest and rest[0] in "+-":
                    op = rest[0]
                    right, rest = _expr(rest[1:])
                    left = left + right if op == "+" else left - right
                return left, rest


            def _term(tokens):
                left, rest = _atom(tokens)
                if rest and rest[0] in "*/":
                    op = rest[0]
                    right, rest = _term(rest[1:])
                    left = left * right if op == "*" else left / right
                return left, rest


            def _atom(tokens):
                head, rest = tokens[0], tokens[1:]
                if head == "(":
                    value, rest = _expr(rest)
                    return value, rest[1:]
                return float(head), rest
        ''',
        "test_calc.py": _T8_TESTS,
    },
    {
        "test_calc.py": _T8_TESTS,
        "test_hidden_calc.py": '''
            import pytest
            from calc import evaluate


            @pytest.mark.parametrize("expr,value", [
                ("8 - 2 - 1 - 1", 4),
                ("100 / 10 / 5", 2),
                ("2 * 3 - 4 / 2", 4),
                ("-3 + 5", 2),
                ("-(2 + 1) * 2", -6),
                ("4 - -2", 6),
                ("((2))", 2),
                ("1.5 * 2", 3),
            ])
            def test_more(expr, value):
                assert evaluate(expr) == pytest.approx(value)


            @pytest.mark.parametrize("expr", ["1 +", "(1 + 2", "2 $ 3", ""])
            def test_errors(expr):
                with pytest.raises((ValueError, IndexError)):
                    evaluate(expr)
        ''',
    },
    {
        "calc.py": '''
            import re

            TOKEN = re.compile(r"\\s*(\\d+(?:\\.\\d+)?|[-+*/()])")


            def tokenize(text):
                pos, tokens = 0, []
                text = text.rstrip()
                while pos < len(text):
                    match = TOKEN.match(text, pos)
                    if not match:
                        raise ValueError(f"bad input at {pos}")
                    tokens.append(match.group(1))
                    pos = match.end()
                return tokens


            def evaluate(text):
                tokens = tokenize(text)
                if not tokens:
                    raise ValueError("empty")
                value, rest = _expr(tokens)
                if rest:
                    raise ValueError("trailing input")
                return value


            def _expr(tokens):
                left, rest = _term(tokens)
                while rest and rest[0] in "+-":
                    op = rest[0]
                    right, rest = _term(rest[1:])
                    left = left + right if op == "+" else left - right
                return left, rest


            def _term(tokens):
                left, rest = _unary(tokens)
                while rest and rest[0] in "*/":
                    op = rest[0]
                    right, rest = _unary(rest[1:])
                    left = left * right if op == "*" else left / right
                return left, rest


            def _unary(tokens):
                if tokens and tokens[0] == "-":
                    value, rest = _unary(tokens[1:])
                    return -value, rest
                return _atom(tokens)


            def _atom(tokens):
                if not tokens:
                    raise ValueError("unexpected end")
                head, rest = tokens[0], tokens[1:]
                if head == "(":
                    value, rest = _expr(rest)
                    if not rest or rest[0] != ")":
                        raise ValueError("missing )")
                    return value, rest[1:]
                if head in "+-*/)":
                    raise ValueError(f"unexpected {head}")
                return float(head), rest
        ''',
    },
)

print("tasks written to", ROOT)
