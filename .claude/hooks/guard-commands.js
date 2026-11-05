// PreToolUse: refuses commands that break this repo's rules. See CLAUDE.md.
const { spawnSync } = require('child_process');
const fs = require('fs');
const path = require('path');

// Shared with the commit-msg hook, so there's one definition of a Claude signature.
function attributionPatterns() {
  const file = path.join(__dirname, '..', '..', '.githooks', 'attribution-patterns.txt');
  return fs
    .readFileSync(file, 'utf8')
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => new RegExp(line, 'iu'));
}

function deny(reason) {
  process.stdout.write(
    JSON.stringify({
      hookSpecificOutput: {
        hookEventName: 'PreToolUse',
        permissionDecision: 'deny',
        permissionDecisionReason: reason,
      },
    }),
  );
}

function deadlockRunning() {
  const result = spawnSync('tasklist', ['/FI', 'IMAGENAME eq deadlock.exe', '/NH'], { encoding: 'utf8' });
  return /deadlock\.exe/i.test(result.stdout ?? '');
}

// The scripts that switch picks, which would cut off a live match. The probe only ever picks the
// server the match is already on.
function switchesPicks(command) {
  if (/live-test\.ps1/i.test(command)) return true;
  return /load-sim\.ps1/i.test(command) && !/-Scenario['",\s]+probe/i.test(command);
}

let input = '';
process.stdin.on('data', (chunk) => (input += chunk));
process.stdin.on('end', () => {
  const command = JSON.parse(input).tool_input?.command ?? '';

  if (/\bgit\b[^\n;|&]*\bpush\b/.test(command) || /\bgit\b[^\n;|&]*\bremote\s+(add|set-url)\b/.test(command) || /\bgh\s+repo\s+create\b/.test(command)) {
    deny('This repo is local only: no remotes and no pushing.');
    return;
  }

  if (/\bgit\b[^\n;|&]*\bcommit\b/.test(command) && attributionPatterns().some((p) => p.test(command))) {
    deny('Commit messages in this repo must not contain Claude attribution. Remove it and commit again.');
    return;
  }

  if (switchesPicks(command) && deadlockRunning()) {
    deny('Deadlock is running, and this script switches picks, which could cut off a live match. Use scripts/game-log.ps1 or load-sim.ps1 -Scenario probe, or ask the user to close the game.');
  }
});
