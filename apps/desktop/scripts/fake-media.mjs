// A local media server for the engine's demo school (SJTU_CANVAS_FAKE_MEDIA),
// the Node counterpart of engine/tests/mock_media.py.
//
//   node scripts/fake-media.mjs [port] [--rate BYTES_PER_SECOND]
//
// GET /media/<name>?size=N[&rate=R] answers N bytes of a fixed pattern with
// Range, ETag and Content-Length, throttled to R bytes per second.
import { createServer } from 'node:http';

const CHUNK = 64 * 1024;
const PERIOD = Buffer.from(Array.from({ length: 251 }, (_, index) => (index * 31 + 7) % 251));

function pattern(start, end) {
  const length = Math.max(0, end - start);
  const offset = start % PERIOD.length;
  const repeats = Math.floor((offset + length) / PERIOD.length) + 1;
  return Buffer.concat(Array.from({ length: repeats }, () => PERIOD)).subarray(offset, offset + length);
}

export function startFakeMedia({ port = 0, rate = 8_000_000 } = {}) {
  const server = createServer((request, response) => {
    const url = new URL(request.url ?? '/', 'http://127.0.0.1');
    if (!url.pathname.startsWith('/media/') || (request.method !== 'GET' && request.method !== 'HEAD')) {
      response.writeHead(404).end();
      return;
    }
    const size = Number(url.searchParams.get('size') ?? '0');
    const speed = Number(url.searchParams.get('rate') ?? String(rate));
    if (!Number.isFinite(size) || size < 0) {
      response.writeHead(400).end();
      return;
    }
    let start = 0;
    let end = size;
    let status = 200;
    const range = /^bytes=(\d*)-(\d*)$/.exec(request.headers.range ?? '');
    if (range && size > 0) {
      start = range[1] ? Number(range[1]) : Math.max(0, size - Number(range[2]));
      end = range[1] && range[2] ? Math.min(size, Number(range[2]) + 1) : size;
      if (start >= size || start >= end) {
        response.writeHead(416, { 'Content-Range': `bytes */${size}` }).end();
        return;
      }
      status = 206;
    }
    const headers = {
      'Content-Type': 'video/mp4',
      'Content-Length': String(end - start),
      'Accept-Ranges': 'bytes',
      ETag: `"demo-${size}"`,
    };
    if (status === 206) {
      headers['Content-Range'] = `bytes ${start}-${end - 1}/${size}`;
    }
    response.writeHead(status, headers);
    if (request.method === 'HEAD') {
      response.end();
      return;
    }
    let position = start;
    const pump = () => {
      if (position >= end || response.destroyed) {
        response.end();
        return;
      }
      const next = Math.min(end, position + CHUNK);
      const ok = response.write(pattern(position, next));
      position = next;
      const wait = speed > 0 ? (CHUNK / speed) * 1000 : 0;
      if (ok) {
        setTimeout(pump, wait);
      } else {
        response.once('drain', () => setTimeout(pump, wait));
      }
    };
    pump();
  });
  return new Promise((resolve) => {
    server.listen(port, '127.0.0.1', () => {
      const address = server.address();
      resolve({ server, origin: `http://127.0.0.1:${address.port}` });
    });
  });
}

if (process.argv[1] && import.meta.url === new URL(`file://${process.argv[1]}`).href) {
  const args = process.argv.slice(2);
  const port = Number(args.find((value) => /^\d+$/.test(value)) ?? '8765');
  const rateIndex = args.indexOf('--rate');
  const rate = rateIndex >= 0 ? Number(args[rateIndex + 1]) : 8_000_000;
  const { origin } = await startFakeMedia({ port, rate });
  console.log(`fake media at ${origin}`);
}
