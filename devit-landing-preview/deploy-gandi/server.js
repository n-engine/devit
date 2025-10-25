import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const distDir = path.join(__dirname, 'dist');

const MIME_TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.mjs': 'application/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.svg': 'image/svg+xml',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.ico': 'image/x-icon'
};

const normalizePath = (requestPath) => {
  if (!requestPath || requestPath === '/') {
    return path.join(distDir, 'index.html');
  }

  const safePath = path.normalize(requestPath).replace(/^(\.\.[/\\])+/, '');
  return path.join(distDir, safePath);
};

const resolveFile = async (filePath) => {
  let candidate = filePath;

  try {
    const stats = await stat(candidate);
    if (stats.isDirectory()) {
      candidate = path.join(candidate, 'index.html');
    }
  } catch {
    candidate = path.join(distDir, 'index.html');
  }

  if (!candidate.startsWith(distDir)) {
    return path.join(distDir, 'index.html');
  }

  return candidate;
};

const server = createServer(async (req, res) => {
  try {
    const urlPath = req.url?.split('?')[0];
    const filePath = await resolveFile(normalizePath(urlPath));
    const ext = path.extname(filePath);
    const mimeType = MIME_TYPES[ext] ?? 'application/octet-stream';

    // IMPORTANT: HTML files should NOT be cached (so new builds are loaded)
    // Assets with hash in filename can be cached for a long time
    const cacheControl = ext === '.html' 
      ? 'no-cache, no-store, must-revalidate' 
      : 'public, max-age=31536000, immutable';  // 1 year for assets

    const data = await readFile(filePath);
    res.writeHead(200, { 
      'Content-Type': mimeType, 
      'Cache-Control': cacheControl 
    });
    res.end(data);
  } catch (error) {
    res.writeHead(500, { 'Content-Type': 'text/plain; charset=utf-8' });
    res.end('Server error');
    console.error('Static server error:', error);
  }
});

const PORT = process.env.PORT ? Number(process.env.PORT) : 8080;
server.listen(PORT, () => {
  console.log(`DevIT landing server listening on http://localhost:${PORT}`);
});
