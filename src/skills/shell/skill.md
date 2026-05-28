# shell

Executes a shell command in the operating system. Use with caution.

## Parameters
- `command` (string, required): The shell command to run (e.g., "npm install express").
- `cwd` (string, optional): Optional subdirectory relative to the workspace directory to run the command in.

## Code
```javascript
const { exec } = await import('child_process')
const { promisify } = await import('util')
const pathModule = await import('path')

const execAsync = promisify(exec)

const BLOCKED = [
  /rm\s+-rf\s+\//,
  /mkfs/,
  /dd\s+if=/,
  /:\(\)\{.*\}/,
  /curl.*\|\s*(ba)?sh/,
  /wget.*\|\s*(ba)?sh/,
]

for (const pattern of BLOCKED) {
  if (pattern.test(command)) {
    throw new Error(`shell: comando bloqueado — ${command}`)
  }
}

// No modo direto (sem taskId), executa no diretório de trabalho atual
const workdir = context.taskId
  ? pathModule.resolve('workspace', context.taskId)
  : process.cwd()

const { stdout, stderr } = await execAsync(command, {
  cwd: cwd ? pathModule.join(workdir, cwd) : workdir,
  timeout: 30000,
}).catch(err => ({ stdout: err.stdout || '', stderr: err.stderr || err.message }))

const output = [stdout, stderr].filter(Boolean).join('\n').trim()
return output ? `$ ${command}\n${output}` : `$ ${command}\n(sem output)`
```
