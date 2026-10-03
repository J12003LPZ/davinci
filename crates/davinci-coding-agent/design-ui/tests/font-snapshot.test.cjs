'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { snapshotFonts } = require('../scripts/font-snapshot.cjs');

test('explicit font setup copies linked directories into a bounded regular inventory', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'design-fonts-'));
  try {
    const source = path.join(root, 'source'), target = path.join(root, 'target');
    fs.mkdirSync(source); fs.mkdirSync(path.join(root, 'shared'));
    fs.writeFileSync(path.join(root, 'shared', 'fixture.ttf'), 'font fixture');
    fs.symlinkSync(path.join(root, 'shared'), path.join(source, 'linked'), process.platform === 'win32' ? 'junction' : 'dir');
    const pin = snapshotFonts([source], target);
    assert.equal(fs.readFileSync(path.join(target, '0/linked/fixture.ttf'), 'utf8'), 'font fixture');
    assert.equal(fs.lstatSync(path.join(target, '0/linked')).isSymbolicLink(), false);
    assert.equal(pin.directory, fs.realpathSync(target));
    assert.match(pin.files['0/linked/fixture.ttf'], /^[a-f0-9]{64}$/);
    fs.symlinkSync(source, path.join(source, 'cycle'), process.platform === 'win32' ? 'junction' : 'dir');
    assert.throws(() => snapshotFonts([source], path.join(root, 'cycle-copy')), /cycle/);
    assert.throws(() => snapshotFonts([path.join(root, 'shared')], path.join(root, 'bounded'), { maxBytes: 2 }), /budget/);
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});
