# file_read

Reads text content from a file inside the task workspace.

## Parameters
- `path` (string, required): The relative path of the file to read (e.g., "src/main.js"). Cannot use ".." to escape the workspace.
- `lines` (string, optional): Optional line range to read (e.g., "1-20" or "50-100").

## Code
```javascript
const fs = await import('fs/promises')
const pathModule = await import('path')

const workdir = context.taskId
  ? pathModule.resolve('workspace', context.taskId)
  : pathModule.resolve('workspace')

// Validação
if (path.includes('..')) {
  throw new Error('file_read: path traversal bloqueado')
}

const fullPath = pathModule.resolve(workdir, path)
if (!fullPath.startsWith(workdir)) {
  throw new Error('file_read: path traversal bloqueado — tentativa de ler fora do workspace')
}

let content = await fs.readFile(fullPath, 'utf-8')

if (lines) {
  const [start, end] = lines.split('-').map(Number)
  content = content.split('\n').slice(start - 1, end).join('\n')
}

return `=== ${path} ===\n${content}`
```
