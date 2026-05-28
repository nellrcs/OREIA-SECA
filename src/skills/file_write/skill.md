# file_write

Writes text content to a file. Access is strictly sandboxed inside the task workspace directory.

## Parameters
- `path` (string, required): The relative path of the file to write (e.g., "src/main.js"). Cannot use ".." to escape the workspace.
- `content` (string, required): The raw text content to write into the file.

## Code
```javascript
const fs = await import('fs/promises')
const pathModule = await import('path')

const workdir = context.taskId
  ? pathModule.resolve('workspace', context.taskId)
  : pathModule.resolve('workspace')

// Validação
if (path.includes('..')) {
  throw new Error(`file_write: path traversal bloqueado — ${path}`)
}

const fullPath = pathModule.resolve(workdir, path)
if (!fullPath.startsWith(workdir)) {
  throw new Error(`file_write: path traversal bloqueado — tentativa de gravar fora do workspace`)
}

await fs.mkdir(pathModule.dirname(fullPath), { recursive: true })
await fs.writeFile(fullPath, content, 'utf-8')

const lines = content.split('\n').length
return `arquivo criado: ${path} (${lines} linhas)`
```
