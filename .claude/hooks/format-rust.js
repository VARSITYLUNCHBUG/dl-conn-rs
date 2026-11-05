// PostToolUse: formats a Rust file after it's edited, so format never drifts between edits.
const { spawnSync } = require('child_process');
const path = require('path');

let input = '';
process.stdin.on('data', (chunk) => (input += chunk));
process.stdin.on('end', () => {
  const file = JSON.parse(input).tool_input?.file_path ?? '';
  if (!file.endsWith('.rs')) return;
  // The session's PATH may not include cargo.
  const rustfmt = path.join(process.env.USERPROFILE ?? process.env.HOME ?? '', '.cargo', 'bin', 'rustfmt');
  const result = spawnSync(rustfmt, ['--edition', '2024', file], { encoding: 'utf8' });
  // A syntax error mid-edit is normal, and the compiler will report it better, so stay quiet.
  if (result.error) process.stderr.write(`rustfmt not run: ${result.error.message}\n`);
});
