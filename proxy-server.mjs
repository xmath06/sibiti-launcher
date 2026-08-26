// CBT Launcher — static file server + reverse proxy untuk frontend CBT.
// Menyajikan folder build/ SPA sekaligus meneruskan semua request /api/*
// (termasuk SSE /monitor/.../stream dan /upload) ke backend yang jalan
// di port 3000. Ini menggantikan `pm2 serve` yang hanya static (tidak ada
// proxy), sehingga frontend same-origin bisa memanggil backend.
//
// Jalankan dengan: bun proxy-server.mjs  (dikelola via pm2 oleh launcher)
// Env opsional: FRONTEND_BUILD, BACKEND_URL, FRONTEND_PORT

import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const BUILD_DIR = process.env.FRONTEND_BUILD
  ? path.resolve(process.env.FRONTEND_BUILD)
  : path.resolve(__dirname, '../frontend/build');

const BACKEND_URL = process.env.BACKEND_URL || 'http://127.0.0.1:3000';
const PORT = Number(process.env.FRONTEND_PORT || 5173);

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
  '.ico': 'image/x-icon',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.ttf': 'font/ttf',
  '.txt': 'text/plain; charset=utf-8',
  '.map': 'application/json'
};

async function exists(p) {
  try {
    const f = Bun.file(p);
    return await f.exists();
  } catch {
    return false;
  }
}

function cleanPath(urlPath) {
  let p = decodeURIComponent(urlPath.split('?')[0]);
  if (p === '/') p = '/index.html';
  const resolved = path.normalize(p).replace(/^(\.\.[/\\])+/, '');
  return path.join(BUILD_DIR, resolved);
}

const server = Bun.serve({
  port: PORT,
  hostname: '0.0.0.0',
  async fetch(req) {
    const url = new URL(req.url);

    // ── Reverse proxy ke backend (same-origin API) ──
    if (
      url.pathname.startsWith('/api') ||
      url.pathname.startsWith('/upload') ||
      url.pathname.startsWith('/monitor')
    ) {
      const target = `${BACKEND_URL}${url.pathname}${url.search}`;
      const res = await fetch(target, {
        method: req.method,
        headers: req.headers,
        body: req.method === 'GET' || req.method === 'HEAD' ? undefined : req.body
      });
      return res;
    }

    // ── Static file ──
    const fp = cleanPath(url.pathname);
    if (await exists(fp)) {
      const ext = fp.slice(fp.lastIndexOf('.'));
      return new Response(Bun.file(fp), {
        headers: { 'content-type': MIME[ext] || 'application/octet-stream' }
      });
    }

    // ── SPA fallback → index.html ──
    const index = path.join(BUILD_DIR, 'index.html');
    if (await exists(index)) {
      return new Response(Bun.file(index), {
        headers: { 'content-type': 'text/html; charset=utf-8' }
      });
    }

    return new Response('Not Found', { status: 404 });
  }
});

console.log(
  `[CBT Launcher] Frontend: ${BUILD_DIR}\n[CBT Launcher] Serving  http://127.0.0.1:${PORT}\n[CBT Launcher] Proxy    /api -> ${BACKEND_URL}`
);
