'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {createArtifactSink} = require('./browser_host.js');

test('screenshots transfer as hashed bounded staging files, without overwriting', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'davinci-browser-transfer-'));
  try {
    const sink = createArtifactSink(directory);
    const bytes = Buffer.from('screenshot fixture');
    const result = await sink(1, {bytes, mediaType: 'image/png'});
    assert.match(result.artifact, /^staging:[a-f0-9]{64}:[a-f0-9]{64}$/);
    assert.equal(result.size, bytes.length);
    const file = path.join(directory, result.artifact.split(':')[1] + '.png');
    assert.deepEqual(fs.readFileSync(file), bytes);
    const second = await sink(2, {bytes, mediaType: 'image/png'});
    assert.notEqual(second.artifact, result.artifact);
    assert.deepEqual(fs.readFileSync(file), bytes);
    await assert.rejects(sink(1, {bytes: Buffer.alloc(4 * 1024 * 1024 + 1), mediaType: 'image/png'}));
  } finally {fs.rmSync(directory, {recursive: true, force: true});}
});

test('staging admission is bounded and host consumption frees a slot', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'davinci-browser-transfer-'));
  try {
    const sink = createArtifactSink(directory);
    const captures = [];
    for (let i = 0; i < 8; i++) captures.push(await sink(1, {bytes: Buffer.from(String(i)), mediaType: 'image/png'}));
    await assert.rejects(sink(1, {bytes: Buffer.from('ninth'), mediaType: 'image/png'}));
    fs.unlinkSync(path.join(directory, captures[0].artifact.split(':')[1] + '.png'));
    await sink(1, {bytes: Buffer.from('ninth'), mediaType: 'image/png'});
  } finally {fs.rmSync(directory, {recursive: true, force: true});}
});
