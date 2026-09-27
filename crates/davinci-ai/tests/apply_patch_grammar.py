"""Exercise the actual advertised grammar with a Lark parser.

This developer check is independent of the Rust executor tests. Install the
test-only dependency with `python -m pip install lark==1.2.2`, then run this file.
It does not establish acceptance by a live provider.
"""

from pathlib import Path
import unittest

from lark import Lark, UnexpectedInput


class PatchGrammarTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        source = (Path(__file__).resolve().parents[1] / "src/apply_patch_grammar.rs").read_text()
        grammar = source.split('r#"', 1)[1].split('"#;', 1)[0]
        # LALR rejects zero-width terminals, including the former /(.*)/.
        cls.parser = Lark(grammar, parser="lalr")

    def test_valid_patch_forms(self):
        patches = [
            "*** Add File: empty.txt\n",
            "*** Add File: lines.txt\n+\n+Unicode β and *** literal content\n",
            "*** Delete File: obsolete.txt\n",
            "*** Update File: code.txt\n@@\n first\n\n-old\n+new\n*** End of File\n",
            "*** Update File: code.txt\n@@ first\n-old\n+new\n@@ last\n-last\n+tail\n",
        ]
        for patch in patches:
            for newline in ("\n", "\r\n"):
                for trailing_newline in (False, True):
                    text = "*** Begin Patch\n" + patch + "*** End Patch"
                    if trailing_newline:
                        text += "\n"
                    text = text.replace("\n", newline)
                    with self.subTest(patch=patch, newline=newline, trailing=trailing_newline):
                        self.parser.parse(text)

    def test_rejects_unadvertised_or_incomplete_hunks(self):
        for body in (
            "*** Update File: code.txt\n@@\n",  # No hunk content.
            "*** Update File: code.txt\n@@\nunprefixed text\n",
            "*** Update File: code.txt\n-old\n+new\n",  # No @@.
            "*** Move to: renamed.txt\n",
            "*** End of File\n",  # No file operation.
            "*** Update File: code.txt\n@@\n-old\n+new\n*** End of File",  # Missing line break.
        ):
            with self.subTest(body=body), self.assertRaises(UnexpectedInput):
                self.parser.parse("*** Begin Patch\n" + body + "*** End Patch")


if __name__ == "__main__":
    unittest.main()
