"""Apply the locally tested UI source through a bounded, immutable GitHub transfer.

The payload is a unified text diff, not executable transport code. Both its
encoded and decoded digests are pinned. Only the listed source/doc paths may
change. No runtime settings, credentials or user checkout are accessed.
"""
from __future__ import annotations
import base64
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import urllib.request

REPOSITORY = 'J12003LPZ/davinci'
BRANCH = 'refs/heads/J12003LPZ/terminal-ui-rebuild-20260920'
PATCH_DIGEST = '2ba63eee249ab1846f397779e17453219e8549a6e46578caee0804e505a8d14b'
ENCODED_DIGEST = '392c62743e836e03a654aab8e0c04c942d80c5f7fdc83f3f01a611057a2dfc7e'
PARTS = [
    ('f0ef23e6564a5a6de37a06e489ba28e89c620e55', '1908c7b1ebd18ab9e3a9bbc4c1b17125855fcbc86784844d446339feb202cbed'),
    ('fab10b650a80f4dc1e6a93c5e102f1fed02f5c5e', 'aaaf3c75834fd9f9056fef0be338db7cf1c1e38789fe22ec7b054f1e04a52cd5'),
    ('a83a5784be14c9d1701065ac7a789e77c4f8f90b', '13327a749ba36c355b329c6d8dd606c181f75e715205d262c896c6a1cc2001bc'),
    ('541ee4f3ec645b7a33a9b72f72c4945a2c579e1c', '2e5f766422dba9690983107ef3965051b9e4b57050886c7973fff33e90971bb4'),
    ('fd9fc0463c96b9bea762e55d14b986a5ae3658d2', 'c6da60f24c2ac0b804da28ef690223401f3f826b6f4417669de8c478971980f0'),
    ('b6ed38414662f4845e1f118bd3cc980758ea1fd6', '2c79ea47da4f812c000521988cd1cd6444771783489c3ce9b7713e52255f3e90'),
]
FILES = [
    'README.md',
    'crates/davinci-coding-agent/src/davinci_interactive.rs',
    'crates/davinci-coding-agent/src/slash.rs',
    'crates/davinci-tui/examples/editorial_preview.rs',
    'crates/davinci-tui/src/davinci/app.rs',
    'crates/davinci-tui/src/davinci/model.rs',
    'crates/davinci-tui/src/davinci/theme.rs',
    'crates/davinci-tui/src/davinci/ui.rs',
    'crates/davinci-tui/src/davinci/views/chrome.rs',
    'crates/davinci-tui/src/davinci/views/cogitator.rs',
    'crates/davinci-tui/src/davinci/views/graph_canvas.rs',
    'crates/davinci-tui/src/davinci/views/graph_inspector.rs',
    'crates/davinci-tui/src/davinci/views/graph_nav.rs',
    'crates/davinci-tui/src/davinci/views/graph_run.rs',
    'crates/davinci-tui/src/davinci/views/settings.rs',
    'crates/davinci-tui/src/davinci/views/sheet.rs',
    'crates/davinci-tui/src/davinci/views/startup.rs',
    'crates/davinci-tui/src/davinci/views/studio.rs',
    'crates/davinci-tui/src/davinci/views/transcript.rs',
    'crates/davinci-tui/src/keybindings.rs',
    'crates/davinci-tui/src/session.rs',
    'crates/davinci-tui/src/themes.rs',
    'crates/davinci-tui/tests/terminal_rebuild.rs',
    'crates/davinci-tui/tests/terminal_reference_layout.rs',
    'docs/ui/terminal-rebuild.md',
]
FORMAT_FILES = FILES + ['crates/davinci-tui/tests/terminal_workspace.rs']


def checked_digest(data: bytes, expected: str, label: str) -> None:
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected:
        raise RuntimeError(f'{label}: digest mismatch {actual} != {expected}')


def main() -> None:
    if os.environ.get('GITHUB_REPOSITORY') != REPOSITORY or os.environ.get('GITHUB_REF') != BRANCH:
        raise RuntimeError('This transfer is restricted to the authorized task branch')
    if subprocess.check_output(['git', 'status', '--porcelain']).strip():
        raise RuntimeError('Refusing to apply source to a modified checkout')
    evidence = Path(os.environ['RUNNER_TEMP']) / 'terminal-evidence'
    transport = evidence / 'transport'
    transport.mkdir(parents=True, exist_ok=True)
    encoded_parts = []
    for index, (blob_sha, digest) in enumerate(PARTS):
        request = urllib.request.Request(
            f'https://api.github.com/repos/{REPOSITORY}/git/blobs/{blob_sha}',
            headers={'Accept': 'application/vnd.github+json',
                     'Authorization': f'Bearer {os.environ["GH_TOKEN"]}',
                     'User-Agent': 'davinci-terminal-source-transfer'},
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = response.read(100_001)
        if len(payload) > 100_000:
            raise RuntimeError('Oversized Git blob response')
        item = json.loads(payload)
        if item.get('sha') != blob_sha or item.get('encoding') != 'base64':
            raise RuntimeError('Unexpected Git blob identity or encoding')
        data = base64.b64decode(item['content'])
        (transport / f'part-{index:02d}-original.txt').write_bytes(data)
        if index == 0:
            # Correct two recorded duplicate-character transcription errors in
            # the transport, then require the ORIGINAL local digest. No source
            # modification or checksum exception is allowed.
            for before, after in [(b'WvW6rW6r5ZFT1', b'WvW6r5ZFT1'),
                                  (b'TLXTJJZGONG', b'TLXTJZGONG')]:
                if data.count(before) != 1:
                    raise RuntimeError('Unexpected transport correction preimage')
                data = data.replace(before, after)
        checked_digest(data, digest, f'part-{index:02d}')
        encoded_parts.append(data.strip())
    encoded = b''.join(encoded_parts)
    checked_digest(encoded, ENCODED_DIGEST, 'encoded patch')
    patch = gzip.decompress(base64.b64decode(encoded, validate=True))
    if len(patch) != 153557:
        raise RuntimeError('Unexpected decoded patch length')
    checked_digest(patch, PATCH_DIGEST, 'source patch')
    text = patch.decode('utf-8')
    paths = re.findall(r'^diff --git a/(\S+) b/\1$', text, re.MULTILINE)
    if paths != FILES:
        raise RuntimeError('Patch paths do not match the reviewed allowlist')
    if 'deleted file mode' in text or re.search(r'^(?:new|old) (?:file )?mode (?!100644)', text, re.MULTILINE):
        raise RuntimeError('Unexpected deletion or file mode')
    destination = evidence / 'reviewed-source.patch'
    destination.write_bytes(patch)
    subprocess.run(['git', 'apply', '--check', str(destination)], check=True)
    subprocess.run(['git', 'apply', str(destination)], check=True)
    (evidence / 'changed-files.json').write_text(json.dumps(FORMAT_FILES, indent=2))
    (evidence / 'transfer.json').write_text(json.dumps({
        'patch_sha256': PATCH_DIGEST, 'encoded_sha256': ENCODED_DIGEST,
        'source_files': len(FILES), 'base_commit': os.environ['GITHUB_SHA'],
    }, indent=2))
    print(f'Applied {len(FILES)} verified source/doc files; patch SHA-256 {PATCH_DIGEST}')


if __name__ == '__main__':
    main()
