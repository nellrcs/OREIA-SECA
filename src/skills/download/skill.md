# skill_download

Downloads a file from a public URL and saves it locally in the designated folder (defaults to the task's workspace root) with automatic filename detection and sanitization.

## Parameters
- `url` (string, required): The absolute URL of the file to download.
- `fileName` (string, optional): Local name for the file (e.g. `report.pdf`). If omitted, it is automatically detected.
- `destFolder` (string, optional): Destination folder relative to the workspace. Defaults to "." (workspace root).

## Code
```javascript
const fs = await import('fs/promises');
const path = await import('path');

// 1. URL parsing and network security checks
let parsedUrl;
try {
  parsedUrl = new URL(url);
} catch (err) {
  throw new Error(`skill_download: invalid URL: ${url}`);
}

const blockedHosts = ['localhost', '127.0.0.1', '0.0.0.0', 'host.docker.internal'];
if (blockedHosts.includes(parsedUrl.hostname) || parsedUrl.hostname.startsWith('192.168.') || parsedUrl.hostname.startsWith('10.')) {
  throw new Error('skill_download: requesting private/local network addresses is blocked for security reasons');
}

// 2. Resolve destination folder and prevent path traversal
const relativeDest = destFolder || '.';
if (relativeDest.includes('..') || (fileName && fileName.includes('..'))) {
  throw new Error('skill_download: path traversal blocked');
}

const workdir = (relativeDest.startsWith('tasks') || relativeDest === 'tasks')
  ? path.resolve()
  : (context && context.taskId ? path.resolve('workspace', context.taskId) : path.resolve('workspace'));

const resolvedFolder = path.resolve(workdir, relativeDest);
if (!resolvedFolder.startsWith(workdir)) {
  throw new Error('skill_download: path traversal blocked');
}

// Ensure destination folder exists
await fs.mkdir(resolvedFolder, { recursive: true });

// 3. Perform HTTP request
const controller = new AbortController();
const id = setTimeout(() => controller.abort(), 60000); // 60s timeout for downloads

try {
  const res = await fetch(parsedUrl.href, {
    signal: controller.signal,
    headers: {
      'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) OREIASECA/0.1.0'
    }
  });
  clearTimeout(id);

  if (!res.ok) {
    throw new Error(`HTTP ${res.status} ${res.statusText}`);
  }

  // 4. Resolve filename
  let resolvedFileName = fileName;
  
  if (!resolvedFileName) {
    const contentDisp = res.headers.get('content-disposition');
    if (contentDisp) {
      // Look for filename="name" or filename*=UTF-8''name
      const match = contentDisp.match(/filename\*?=["']?(?:[a-zA-Z0-9\-#%]+'')?([^"'\r\n;]+)["']?/i);
      if (match && match[1]) {
        resolvedFileName = decodeURIComponent(match[1]);
      }
    }
  }

  if (!resolvedFileName) {
    const pathName = parsedUrl.pathname;
    const lastSegment = pathName.substring(pathName.lastIndexOf('/') + 1);
    if (lastSegment && lastSegment.includes('.')) {
      resolvedFileName = lastSegment;
    }
  }

  if (!resolvedFileName) {
    resolvedFileName = `download_${Date.now()}`;
  }

  // Sanitize filename to prevent any malicious directory escape
  resolvedFileName = path.basename(resolvedFileName);

  const finalFilePath = path.join(resolvedFolder, resolvedFileName);

  // 5. Read body and save file
  const arrayBuffer = await res.arrayBuffer();
  const buffer = Buffer.from(arrayBuffer);
  await fs.writeFile(finalFilePath, buffer);

  const relativeSavePath = path.relative(path.resolve(), finalFilePath).replace(/\\/g, '/');
  return `Successfully downloaded "${url}" and saved to "${relativeSavePath}" (${buffer.length} bytes).`;

} catch (err) {
  clearTimeout(id);
  throw new Error(`skill_download failed: ${err.message}`);
}
```
