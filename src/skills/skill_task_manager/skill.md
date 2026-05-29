# skill_task_manager

Manages past task workspaces, allowing the AI to list all active workspaces, search for keywords within task goals and descriptions, list recursive file trees of specific task folders, and securely copy files from a past task workspace into the current active task workspace.

Example XML to list all past tasks:
<action name="skill_task_manager">
  <param name="action">list</param>
</action>

Example XML to search for tasks regarding "git":
<action name="skill_task_manager">
  <param name="action">search</param>
  <param name="query">git</param>
</action>

Example XML to copy a script from a past task to the current workspace:
<action name="skill_task_manager">
  <param name="action">copy</param>
  <param name="taskId">task-ccfe54b3</param>
  <param name="filePath">index.js</param>
  <param name="destPath">scripts/old_index.js</param>
</action>

## Parameters
- `action` (string, required): The task manager operation to perform ("list", "search", "files", "copy").
- `query` (string, optional): A text search query (required for "search").
- `taskId` (string, optional): The target task ID to query files or copy from (required for "files" and "copy").
- `filePath` (string, optional): Relative file path of the source file inside the target task workspace (required for "copy").
- `destPath` (string, optional): Relative destination path inside the current active task workspace (required for "copy").

## Code
```javascript
const fs = await import('fs/promises');
const path = await import('path');

const act = action.toLowerCase();

// 1. Prevenir Path Traversal nos parâmetros críticos
if (taskId && taskId.includes('..')) {
  throw new Error('TaskManager: path traversal blocked in "taskId"');
}
if (filePath && filePath.includes('..')) {
  throw new Error('TaskManager: path traversal blocked in "filePath"');
}
if (destPath && destPath.includes('..')) {
  throw new Error('TaskManager: path traversal blocked in "destPath"');
}

const workspaceRoot = path.resolve('workspace');

// Certificar que a pasta raiz workspace/ exista
await fs.mkdir(workspaceRoot, { recursive: true });

// Ações válidas: list, search, files, copy
if (act === 'list') {
  try {
    const folders = await fs.readdir(workspaceRoot);
    const taskDetails = [];

    for (const folder of folders) {
      const folderPath = path.join(workspaceRoot, folder);
      const stat = await fs.stat(folderPath);
      if (!stat.isDirectory()) continue;

      const descFile = path.join(folderPath, 'sobre_a_tarefa.md');
      let goal = '(sem descrição disponível)';
      let date = stat.birthtime.toLocaleString('pt-BR');
      let statusStr = 'unknown';

      try {
        const descContent = await fs.readFile(descFile, 'utf8');
        
        // Tentar extrair objetivo principal via regex
        const goalMatch = descContent.match(/>\s*(.+)/);
        if (goalMatch && goalMatch[1]) {
          goal = goalMatch[1].trim();
        }
        
        // Tentar extrair data
        const dateMatch = descContent.match(/-\s+\*\*Criado em:\*\*\s*(.+)/);
        if (dateMatch && dateMatch[1]) {
          date = dateMatch[1].trim();
        }
        
        // Tentar extrair status inicial/atual
        const statusMatch = descContent.match(/-\s+\*\*Status Inicial:\*\*\s*\`?([\w_-]+)\`?/);
        if (statusMatch && statusMatch[1]) {
          statusStr = statusMatch[1].trim();
        }
      } catch (e) {
        // Arquivo sobre_a_tarefa.md não existe ou falhou ao ler, usar heurísticas/defaults
      }

      taskDetails.push(`- **${folder}**: "${goal}"\n  *Criada em:* ${date} | *Status:* \`${statusStr}\``);
    }

    if (!taskDetails.length) {
      return 'Nenhum espaço de trabalho de tarefa foi encontrado no diretório workspace/.';
    }
    return `Espaços de trabalho de tarefas encontrados:\n\n${taskDetails.join('\n')}`;
  } catch (err) {
    throw new Error(`TaskManager list failed: ${err.message}`);
  }
}

if (act === 'search') {
  if (!query) {
    throw new Error('TaskManager: parameter "query" is required for "search" action.');
  }
  const cleanQuery = query.toLowerCase();

  try {
    const folders = await fs.readdir(workspaceRoot);
    const matchedDetails = [];

    for (const folder of folders) {
      const folderPath = path.join(workspaceRoot, folder);
      const stat = await fs.stat(folderPath);
      if (!stat.isDirectory()) continue;

      const descFile = path.join(folderPath, 'sobre_a_tarefa.md');
      let hasMatch = false;
      let goal = '(sem descrição)';
      let date = stat.birthtime.toLocaleString('pt-BR');
      let statusStr = 'unknown';

      try {
        const descContent = await fs.readFile(descFile, 'utf8');
        if (descContent.toLowerCase().includes(cleanQuery)) {
          hasMatch = true;
        }

        const goalMatch = descContent.match(/>\s*(.+)/);
        if (goalMatch && goalMatch[1]) {
          goal = goalMatch[1].trim();
        }
        const dateMatch = descContent.match(/-\s+\*\*Criado em:\*\*\s*(.+)/);
        if (dateMatch && dateMatch[1]) {
          date = dateMatch[1].trim();
        }
        const statusMatch = descContent.match(/-\s+\*\*Status Inicial:\*\*\s*\`?([\w_-]+)\`?/);
        if (statusMatch && statusMatch[1]) {
          statusStr = statusMatch[1].trim();
        }
      } catch (e) {
        // Se a pasta contiver o termo de query no próprio ID da pasta
        if (folder.toLowerCase().includes(cleanQuery)) {
          hasMatch = true;
        }
      }

      if (hasMatch) {
        matchedDetails.push(`- **${folder}**: "${goal}"\n  *Criada em:* ${date} | *Status:* \`${statusStr}\``);
      }
    }

    if (!matchedDetails.length) {
      return `Nenhuma tarefa correspondente a "${query}" foi encontrada.`;
    }
    return `Tarefas correspondentes a "${query}":\n\n${matchedDetails.join('\n')}`;
  } catch (err) {
    throw new Error(`TaskManager search failed: ${err.message}`);
  }
}

if (act === 'files') {
  if (!taskId) {
    throw new Error('TaskManager: parameter "taskId" is required for "files" action.');
  }

  const targetWorkdir = path.join(workspaceRoot, taskId);
  try {
    // Verificar se o diretório existe e está contido em workspace
    const stat = await fs.stat(targetWorkdir);
    if (!stat.isDirectory()) {
      throw new Error(`Task workspace "${taskId}" is not a directory.`);
    }

    // Função recursiva simples para ler a árvore de arquivos
    async function getFileTree(dir, basePath) {
      const entries = await fs.readdir(dir, { withFileTypes: true });
      const tree = [];

      for (const entry of entries) {
        const relative = path.relative(basePath, path.join(dir, entry.name)).replace(/\\/g, '/');
        if (entry.isDirectory()) {
          // Ignorar pastas de sistema como .git ou node_modules para não sobrecarregar
          if (entry.name === '.git' || entry.name === 'node_modules') {
            tree.push(`- [Dir] ${relative}/ (ignorado)`);
            continue;
          }
          tree.push(`- [Dir] ${relative}/`);
          const subTree = await getFileTree(path.join(dir, entry.name), basePath);
          tree.push(...subTree);
        } else {
          tree.push(`- [File] ${relative}`);
        }
      }
      return tree;
    }

    const fileList = await getFileTree(targetWorkdir, targetWorkdir);
    if (!fileList.length) {
      return `O espaço de trabalho da tarefa "${taskId}" está vazio.`;
    }
    return `Arquivos contidos no espaço de trabalho de "${taskId}":\n\n${fileList.join('\n')}`;
  } catch (err) {
    if (err.code === 'ENOENT') {
      throw new Error(`Task workspace "${taskId}" was not found.`);
    }
    throw new Error(`TaskManager files failed: ${err.message}`);
  }
}

if (act === 'copy') {
  if (!taskId) {
    throw new Error('TaskManager: parameter "taskId" is required for "copy" action.');
  }
  if (!filePath) {
    throw new Error('TaskManager: parameter "filePath" is required for "copy" action.');
  }
  if (!destPath) {
    throw new Error('TaskManager: parameter "destPath" is required for "copy" action.');
  }

  const srcWorkdir = path.join(workspaceRoot, taskId);
  const srcFile = path.join(srcWorkdir, filePath);
  
  const currentTaskId = context && context.taskId;
  const destWorkdir = currentTaskId ? path.join(workspaceRoot, currentTaskId) : workspaceRoot;
  const destFile = path.join(destWorkdir, destPath);

  // Verificações rígidas de contenção de segurança
  if (!srcFile.startsWith(workspaceRoot)) {
    throw new Error('TaskManager: security violation - source path escapes workspace root');
  }
  if (!destFile.startsWith(workspaceRoot)) {
    throw new Error('TaskManager: security violation - destination path escapes workspace root');
  }

  try {
    // 1. Garantir que o workspace destino existe
    await fs.mkdir(path.dirname(destFile), { recursive: true });

    // 2. Copiar o arquivo fisicamente
    await fs.copyFile(srcFile, destFile);

    const relativeDest = path.relative(path.resolve(), destFile).replace(/\\/g, '/');
    return `Successfully copied file "${filePath}" from task "${taskId}" to active workspace at "${relativeDest}".`;
  } catch (err) {
    if (err.code === 'ENOENT') {
      throw new Error(`TaskManager: Source file "${filePath}" from task "${taskId}" not found.`);
    }
    throw new Error(`TaskManager copy failed: ${err.message}`);
  }
}

throw new Error(`TaskManager: Unknown action: "${action}"`);
```
