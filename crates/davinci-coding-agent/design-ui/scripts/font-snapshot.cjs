'use strict';
// Used only by explicit trusted setup, never by requests or generated code.
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');

function snapshotFonts(directories, destination, { maxBytes = 1024 * 1024 * 1024 } = {}) {
  const files = {}; let entries = 0, bytes = 0;
  fs.mkdirSync(destination); // Require a new destination, preserving failed setup.
  function copy(source, target, ancestors) {
    if (++entries > 5000) throw new Error('Font entry budget exceeded');
    const real = fs.realpathSync(source), metadata = fs.statSync(real);
    if (metadata.isDirectory()) {
      if (ancestors.has(real)) throw new Error('Font directory cycle');
      const next = new Set([...ancestors, real]);
      fs.mkdirSync(target);
      for (const name of fs.readdirSync(real).sort()) copy(path.join(real, name), path.join(target, name), next);
    } else if (metadata.isFile()) {
      bytes += metadata.size;
      if (metadata.size > 64 * 1024 * 1024 || bytes > maxBytes) throw new Error('Font byte budget exceeded');
      // Links are resolved during setup. Runtime verification still rejects links.
      fs.copyFileSync(real, target, fs.constants.COPYFILE_EXCL);
      files[path.relative(destination, target).split(path.sep).join('/')] = crypto.createHash('sha256').update(fs.readFileSync(target)).digest('hex');
    } else throw new Error('Nonregular font entry');
  }
  directories.forEach((directory, index) => copy(directory, path.join(destination, String(index)), new Set()));
  if (!Object.keys(files).length) throw new Error('No installed fonts to pin');
  return { directory: fs.realpathSync(destination), files };
}
module.exports = { snapshotFonts };
