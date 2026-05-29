# skill_git

Manages Git repository operations inside the task workspace cleanly, including cloning, pulling updates, and checking status, by delegating calls through the central shell action handler.

Example XML to clone a repository:
<action name="skill_git">
  <param name="action">clone</param>
  <param name="repoUrl">https://github.com/expressjs/express.git</param>
  <param name="destFolder">express-repo</param>
</action>

## Parameters
- `action` (string, required): The Git operation to perform ("clone", "pull", "status").
- `repoUrl` (string, optional): The absolute Git repository URL (e.g. `https://github.com/expressjs/express.git`). Required for "clone".
- `destFolder` (string, optional): Subdirectory folder name relative to the active workspace directory to target. Traversal via ".." is blocked.

## Code
```javascript
const path = await import('path');
const { pathToFileURL } = await import('url');

// 1. Carregar dinamicamente o actionRegistry para acessar o handler do shell
const registryPath = path.resolve('src/actions/registry.js');
const { actionRegistry } = await import(pathToFileURL(registryPath).href);
const shellHandler = actionRegistry.handlers.get('shell');

const act = action.toLowerCase();

// 2. Prevenir vulnerabilidades de Path Traversal
if (destFolder && destFolder.includes('..')) {
  throw new Error('Git: path traversal blocked');
}

// 3. Verificar instalação do Git
try {
  await shellHandler.run({ command: 'git --version' }, context);
} catch (err) {
  throw new Error('Git is not installed or not available in the system PATH.');
}

if (act === 'clone') {
  if (!repoUrl) {
    throw new Error('Git: "repoUrl" parameter is required for clone action.');
  }

  // Se destFolder for especificado, passamos no comando
  const cloneCmd = destFolder ? `git clone ${repoUrl} "${destFolder}"` : `git clone ${repoUrl}`;
  try {
    const res = await shellHandler.run({ command: cloneCmd }, context);
    return res;
  } catch (err) {
    throw new Error(`Git clone failed: ${err.message}`);
  }
}

if (act === 'pull') {
  try {
    const res = await shellHandler.run({
      command: 'git pull',
      cwd: destFolder || undefined
    }, context);
    return res;
  } catch (err) {
    throw new Error(`Git pull failed: ${err.message}`);
  }
}

if (act === 'status') {
  try {
    const res = await shellHandler.run({
      command: 'git status',
      cwd: destFolder || undefined
    }, context);
    return res;
  } catch (err) {
    throw new Error(`Git status failed: ${err.message}`);
  }
}

throw new Error(`Unknown Git action: "${action}"`);
```
