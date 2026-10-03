#!/usr/bin/env python3
"""Prepare source-only review-fix trees. Never update a feature branch or main."""
from __future__ import annotations
import json
import os
from pathlib import Path
import subprocess

ROOT = Path.cwd()
TEMP = Path(os.environ['RUNNER_TEMP'])
HEAD85 = '06bc2f8aa93fee58568cd37fe68ef5a6d24810e3'
HEAD86 = 'a7a59af4cc2781296c1f69f7dc214da784790bce'
HEAD84 = 'b30783f610597f8448a35097ef5e1ad4c55e509d'
PREFIX = 'fix/review-prepared-20261003-v2'
CHANGES = json.loads((ROOT / 'changes.json').read_text(encoding='utf-8'))


def run(*args: str, cwd: Path = ROOT, capture: bool = False, input: str | None = None) -> str:
    completed = subprocess.run(args, cwd=cwd, input=input, text=True, check=True,
                               stdout=subprocess.PIPE if capture else None)
    return completed.stdout.strip() if capture else ''


def apply(directory: Path, groups: set[str]) -> list[str]:
    buffers: dict[str, str] = {}
    for change in CHANGES:
        if change['group'] not in groups:
            continue
        relative = change['path']
        path = directory / relative
        if path.is_symlink() or '..' in Path(relative).parts or Path(relative).is_absolute():
            raise RuntimeError(f'Unsafe source path: {relative}')
        if 'create' in change:
            if path.exists() or relative in buffers:
                raise RuntimeError(f'New test already exists: {relative}')
            buffers[relative] = change['create']
            continue
        text = buffers.get(relative)
        if text is None:
            text = path.read_text(encoding='utf-8')
        if text.count(change['before']) != 1:
            raise RuntimeError(f'Expected exactly one reviewed anchor: {relative}')
        buffers[relative] = text.replace(change['before'], change['after'], 1)
    if 'integration' in groups:
        # Keep the regression module at file end, following the repository's
        # Rust layout rather than placing production items after a test module.
        relative = 'crates/davinci-coding-agent/src/design/model.rs'
        text = buffers[relative]
        marker = '#[cfg(test)]\nmod service_tier_regression_tests {'
        if text.count(marker) != 1:
            raise RuntimeError('Expected one Design tier regression module')
        start = text.index(marker)
        end = text.index('impl DesignModel for SubscriptionModel {', start)
        tests = text[start:end].strip()
        buffers[relative] = (text[:start] + text[end:]).rstrip() + '\n\n' + tests + '\n'
    # All anchors must be valid before the disposable worktree is changed.
    for relative, content in buffers.items():
        (directory / relative).write_text(content, encoding='utf-8', newline='\n')
    run('cargo', 'fmt', '--all', cwd=directory)
    unexpected = set(run('git', 'diff', '--name-only', cwd=directory, capture=True).splitlines()) - buffers.keys()
    if unexpected:
        raise RuntimeError(f'Formatting changed unrelated source: {sorted(unexpected)}')
    run('git', 'add', '--', *sorted(buffers), cwd=directory)
    run('git', 'diff', '--cached', '--check', cwd=directory)
    return sorted(buffers)


def publish(directory: Path, parents: list[str], suffix: str) -> tuple[str, str]:
    tree = run('git', 'write-tree', cwd=directory, capture=True)
    argv = ['git', 'commit-tree', tree]
    for parent in parents:
        argv += ['-p', parent]
    commit = run(*argv, cwd=directory, capture=True,
                 input=f'Prepare source-only {suffix} review fixes for validation\n')
    # New, dedicated scratch refs only. No force push and no feature-ref writes.
    run('git', 'push', 'origin', f'{commit}:refs/heads/{PREFIX}-{suffix}', cwd=directory)
    print(json.dumps({'result': suffix, 'commit': commit, 'tree': tree}), flush=True)
    with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as out:
        out.write(f'{suffix}_commit={commit}\n{suffix}_tree={tree}\n')
    return commit, tree


def worktree(name: str, commit: str) -> Path:
    destination = TEMP / f'davinci-review-{name}'
    run('git', 'worktree', 'add', '--detach', str(destination), commit)
    if run('git', 'rev-parse', 'HEAD', cwd=destination, capture=True) != commit:
        raise RuntimeError('Unexpected checkout revision')
    return destination


def main() -> None:
    run('git', 'config', 'user.name', 'github-actions[bot]')
    run('git', 'config', 'user.email', '41898282+github-actions[bot]@users.noreply.github.com')
    run('git', 'fetch', '--no-tags', 'origin', HEAD85, HEAD86, HEAD84)
    fast = worktree('pr86', HEAD86)
    apply(fast, {'pr86'})
    fast_commit, _ = publish(fast, [HEAD86], 'pr86')
    design = worktree('pr85', HEAD85)
    # Git's three-way merge preserves both feature implementations; conflicts stop.
    run('git', 'merge', '--no-commit', '--no-ff', fast_commit, cwd=design)
    apply(design, {'pr85', 'integration'})
    design_commit, _ = publish(design, [HEAD85, fast_commit], 'pr85')
    combined = worktree('combined', design_commit)
    run('git', 'merge', '--no-commit', '--no-ff', HEAD84, cwd=combined)
    run('git', 'diff', '--cached', '--check', cwd=combined)
    publish(combined, [design_commit, HEAD84], 'combined')


if __name__ == '__main__':
    main()
