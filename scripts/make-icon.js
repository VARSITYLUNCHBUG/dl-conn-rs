// Draws the app icon and writes assets/dl-rs.ico (for the exe) and assets/icon-64.rgba (for the
// window). Run from the repo root with node when the design changes. The results are checked in, so
// a normal build doesn't need this.
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const BACKGROUND = [28, 100, 216];
const MARK = [255, 255, 255];
const SAMPLES = 4;

// Coverage of the design at a point, in a unit square: a dot sending two arcs up and to the right,
// on a rounded square.
function shape(x, y) {
  const corner = 0.2;
  const dx = Math.max(Math.abs(x - 0.5) - (0.5 - corner), 0);
  const dy = Math.max(Math.abs(y - 0.5) - (0.5 - corner), 0);
  if (dx * dx + dy * dy > corner * corner) return 'empty';
  const cx = 0.3;
  const cy = 0.7;
  const distance = Math.hypot(x - cx, y - cy);
  if (distance < 0.085) return 'mark';
  if (x >= cx && y <= cy) {
    if (distance > 0.24 && distance < 0.33) return 'mark';
    if (distance > 0.44 && distance < 0.53) return 'mark';
  }
  return 'fill';
}

function draw(size) {
  const pixels = Buffer.alloc(size * size * 4);
  for (let row = 0; row < size; row++) {
    for (let column = 0; column < size; column++) {
      let fill = 0;
      let mark = 0;
      for (let a = 0; a < SAMPLES; a++) {
        for (let b = 0; b < SAMPLES; b++) {
          const kind = shape((column + (a + 0.5) / SAMPLES) / size, (row + (b + 0.5) / SAMPLES) / size);
          if (kind === 'fill') fill++;
          if (kind === 'mark') mark++;
        }
      }
      const covered = fill + mark;
      const at = (row * size + column) * 4;
      for (let channel = 0; channel < 3; channel++) {
        pixels[at + channel] = covered ? Math.round((BACKGROUND[channel] * fill + MARK[channel] * mark) / covered) : 0;
      }
      pixels[at + 3] = Math.round((255 * covered) / (SAMPLES * SAMPLES));
    }
  }
  return pixels;
}

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc(buffer) {
  let c = 0xffffffff;
  for (const byte of buffer) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type), data]);
  const check = Buffer.alloc(4);
  check.writeUInt32BE(crc(body));
  return Buffer.concat([length, body, check]);
}

function png(size, pixels) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8;
  header[9] = 6;
  const rows = Buffer.alloc((size * 4 + 1) * size);
  for (let row = 0; row < size; row++) {
    pixels.copy(rows, row * (size * 4 + 1) + 1, row * size * 4, (row + 1) * size * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', zlib.deflateSync(rows, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

const sizes = [16, 24, 32, 48, 64, 256];
const images = sizes.map((size) => png(size, draw(size)));
const directory = Buffer.alloc(6 + 16 * sizes.length);
directory.writeUInt16LE(1, 2);
directory.writeUInt16LE(sizes.length, 4);
let offset = directory.length;
sizes.forEach((size, index) => {
  const entry = 6 + 16 * index;
  directory[entry] = size === 256 ? 0 : size;
  directory[entry + 1] = size === 256 ? 0 : size;
  directory.writeUInt16LE(1, entry + 4);
  directory.writeUInt16LE(32, entry + 6);
  directory.writeUInt32LE(images[index].length, entry + 8);
  directory.writeUInt32LE(offset, entry + 12);
  offset += images[index].length;
});

const assets = path.join(__dirname, '..', 'assets');
fs.mkdirSync(assets, { recursive: true });
fs.writeFileSync(path.join(assets, 'dl-rs.ico'), Buffer.concat([directory, ...images]));
fs.writeFileSync(path.join(assets, 'icon-64.rgba'), draw(64));
