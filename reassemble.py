from pathlib import Path
import hashlib
root = Path(__file__).resolve().parent
output = root / "claude-code-davinci-research.zip"
parts = sorted(root.glob(output.name + ".part*"))
if len(parts) != 4:
    raise SystemExit("Expected all four ZIP parts in this directory")
digest = hashlib.sha256()
with output.open("wb") as target:
    for part in parts:
        with part.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                target.write(chunk)
                digest.update(chunk)
expected = "60e2806f7f4bc7de1ddeb9f65339d4a194414e55ffb3abec104dcf7cc25dcaee"
if digest.hexdigest() != expected:
    raise SystemExit("Checksum mismatch; download the parts again")
print(f"Verified: {output}")
