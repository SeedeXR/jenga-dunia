// A loopback Terrarium tile server for the e2e suite: `/t/{z}/{x}/{y}.png` answers a 256² RGB
// PNG whose height is 1000 m + 0.01 × (tile row × 256 + y), so a fetched DEM is checkable, and
// a matching `If-None-Match` gets a 304 like the real bucket. No dependencies: the PNG is
// written by hand (stored deflate via zlib, CRC32 per chunk).
import { createServer, type Server } from "node:http";
import { deflateSync } from "node:zlib";

const CRC_TABLE = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf: Buffer): number {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type: string, data: Buffer): Buffer {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

/** A 256² Terrarium tile for tile row `ty`. */
export function terrariumTile(ty: number): Buffer {
  const size = 256;
  const raw = Buffer.alloc((size * 3 + 1) * size);
  for (let y = 0; y < size; y++) {
    raw[y * (size * 3 + 1)] = 0; // filter: none
    const v = 1000 + 0.01 * (ty * size + y) + 32768;
    const r = Math.floor(v / 256);
    const g = Math.floor(v - r * 256);
    for (let x = 0; x < size; x++) {
      const o = y * (size * 3 + 1) + 1 + x * 3;
      raw[o] = r;
      raw[o + 1] = g;
      raw[o + 2] = 0;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // RGB
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

/** Start the server; returns its base URL (`http://127.0.0.1:<port>/t`) and the handle. */
export async function startTileServer(): Promise<{ base: string; hits: { count: number; conditional: number }; server: Server }> {
  const hits = { count: 0, conditional: 0 };
  const server = createServer((req, res) => {
    const m = /^\/t\/(\d+)\/(\d+)\/(\d+)\.png$/.exec(req.url ?? "");
    if (!m) {
      res.writeHead(404).end("not found");
      return;
    }
    hits.count++;
    if (req.headers["if-none-match"] === '"e2e"') {
      hits.conditional++;
      res.writeHead(304, { ETag: '"e2e"' }).end();
      return;
    }
    const body = terrariumTile(Number(m[3]));
    res.writeHead(200, { "Content-Type": "image/png", "Content-Length": body.length, ETag: '"e2e"' }).end(body);
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const addr = server.address();
  const port = typeof addr === "object" && addr ? addr.port : 0;
  return { base: `http://127.0.0.1:${port}/t`, hits, server };
}
