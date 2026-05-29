// test/skills.js
import path from 'path'
import fs   from 'fs/promises'
import { actionRegistry } from '../src/actions/registry.js'
import { getExecutorSystemPrompt, getDirectSystemPrompt } from '../src/maker/prompts.js'

// ─── Helpers de output ────────────────────────────────────────────────────────
const c = {
  reset:  '\x1b[0m',
  green:  '\x1b[32m',
  red:    '\x1b[31m',
  yellow: '\x1b[33m',
  cyan:   '\x1b[36m',
  gray:   '\x1b[90m',
  bold:   '\x1b[1m',
}

let passed = 0, failed = 0

function section(name) {
  console.log(`\n${c.cyan}${c.bold}══ ${name} ══${c.reset}`)
}

async function test(name, fn) {
  try {
    await fn()
    console.log(`  ${c.green}✓${c.reset} ${name}`)
    passed++
  } catch (err) {
    console.log(`  ${c.red}✗${c.reset} ${name}`)
    console.log(`    ${c.red}${err.message}${c.reset}`)
    failed++
  }
}

function assert(condition, msg) {
  if (!condition) throw new Error(msg ?? 'assertion falhou')
}

function assertEqual(a, b, msg) {
  if (a !== b) throw new Error(msg ?? `esperado ${JSON.stringify(b)}, recebeu ${JSON.stringify(a)}`)
}

// ─── Testes ───────────────────────────────────────────────────────────────────

section('Carregamento de Habilidades Dinâmicas (Skills)')

await test('carrega skills padrão com sucesso', async () => {
  // Inicialmente tem as 3 estáticas
  assertEqual(actionRegistry.handlers.has('shell'), true, 'deve ter shell')
  assertEqual(actionRegistry.handlers.has('file_write'), true, 'deve ter file_write')
  assertEqual(actionRegistry.handlers.has('file_read'), true, 'deve ter file_read')

  // Carrega dinâmicas
  await actionRegistry.loadSkills()

  assertEqual(actionRegistry.handlers.has('skill_fetch'), true, 'deve ter carregado skill_fetch')
  assertEqual(actionRegistry.handlers.has('skill_weather'), true, 'deve ter carregado skill_weather')
})

await test('getActionsSchema retorna schemas válidos com descrições e parâmetros em inglês', () => {
  const schemas = actionRegistry.getActionsSchema()
  assert(schemas.length >= 5, `esperado pelo menos 5 ações registradas, recebeu ${schemas.length}`)

  const skillFetch = schemas.find(s => s.name === 'skill_fetch')
  assert(!!skillFetch, 'deve encontrar o schema de skill_fetch')
  assertEqual(skillFetch.description, 'Fetches raw text or HTML content from a public URL via HTTP request.')
  assertEqual(skillFetch.params.url.required, true)
  assertEqual(skillFetch.params.url.type, 'string')
})

await test('getExecutorSystemPrompt e getDirectSystemPrompt injetam metadados em inglês', () => {
  const schemas = actionRegistry.getActionsSchema()
  const execPrompt = getExecutorSystemPrompt(schemas)
  const directPrompt = getDirectSystemPrompt(schemas)

  // Verifica presença de termos em inglês do prompt do executor
  assert(execPrompt.includes('AVAILABLE ACTIONS:'), 'prompt do executor deve conter AVAILABLE ACTIONS')
  assert(execPrompt.includes('skill_fetch'), 'prompt deve conter a skill_fetch')
  assert(execPrompt.includes('skill_weather'), 'prompt deve conter a skill_weather')
  assert(execPrompt.includes('Brazilian Portuguese (pt-BR)'), 'deve instruir para responder em pt-BR')

  // Verifica prompt direto
  assert(directPrompt.includes('AVAILABLE ACTIONS:'), 'prompt direto deve conter AVAILABLE ACTIONS')
  assert(directPrompt.includes('skill_fetch'), 'prompt direto deve conter skill_fetch')
})

await test('carrega e executa skill Markdown (.md) com sucesso', async () => {
  const tempMdPath = path.resolve('src/skills/skill_temp_math.md')
  
  const mdContent = `# skill_temp_math

Executes a dynamic mathematical sum.

## Parameters
- \`a\` (number, required): The first number
- \`b\` (number, required): The second number

## Code
\`\`\`javascript
const result = Number(a) + Number(b);
return \`O resultado de \${a} + \${b} é \${result}\`;
\`\`\`
`

  await fs.writeFile(tempMdPath, mdContent, 'utf8')

  try {
    // Recarrega as skills
    await actionRegistry.loadSkills()

    // Verifica se registrou a nova skill
    const handler = actionRegistry.handlers.get('skill_temp_math')
    assert(!!handler, 'deve encontrar o handler de skill_temp_math')
    assertEqual(handler.constructor.description, 'Executes a dynamic mathematical sum.')
    assertEqual(handler.constructor.params.a.type, 'number')
    assertEqual(handler.constructor.params.a.required, true)
    
    // Executa a skill
    const res = await handler.run({ a: 10, b: 20 }, { taskId: 'test' })
    assertEqual(res, 'O resultado de 10 + 20 é 30')
  } finally {
    // Remove o arquivo temporário
    await fs.unlink(tempMdPath).catch(() => {})
  }
})

section('Segurança e Isolamento de Skills')

await test('skill_fetch bloqueia conexões locais por segurança', async () => {
  const handler = actionRegistry.handlers.get('skill_fetch')
  assert(!!handler, 'handler do fetch deve estar disponível')

  try {
    await handler.run({ url: 'http://localhost:3000' }, { taskId: 'test' })
    assert(false, 'deveria ter falhado para localhost')
  } catch (err) {
    assert(err.message.includes('blocked for security reasons'), 'deve indicar bloqueio de segurança')
  }

  try {
    await handler.run({ url: 'http://127.0.0.1/abc' }, { taskId: 'test' })
    assert(false, 'deveria ter falhado para 127.0.0.1')
  } catch (err) {
    assert(err.message.includes('blocked for security reasons'), 'deve indicar bloqueio de segurança')
  }
})

await test('skill_weather executa com sucesso e retorna condições meteorológicas', async () => {
  const handler = actionRegistry.handlers.get('skill_weather')
  assert(!!handler, 'handler do clima deve estar disponível')

  const res = await handler.run({ city: 'Sao Paulo' }, { taskId: 'test' })
  assert(res.includes('Weather for Sao Paulo'), 'deve retornar dados do clima')
})

section('Agendador Cron Dinâmico (Docker/Local)')

await test('skill_cron_manager gerencia rotinas, faz parse textual de horários e gera arquivos crontab', async () => {
  const handler = actionRegistry.handlers.get('skill_cron_manager')
  assert(!!handler, 'handler do cron manager deve estar disponível')

  const testRoutineId = 'test_weather_daily'
  
  // 1. Cadastra uma nova rotina usando linguagem natural
  const addRes = await handler.run({
    action: 'add',
    routineId: testRoutineId,
    promptText: 'consulte o clima',
    textSchedule: 'todos os dias as 14 horas'
  }, { taskId: 'test' })

  assert(addRes.includes('sucesso') || addRes.includes('salva localmente'), 'deve retornar mensagem de sucesso ou salvamento local')

  // 2. Valida se salvou no JSON de rotinas
  const workdir = path.resolve('workspace', 'test')
  const ROUTINES_FILE = path.join(workdir, 'cron_routines.json')
  const routinesContent = await fs.readFile(ROUTINES_FILE, 'utf8')
  const routines = JSON.parse(routinesContent)
  
  assert(!!routines[testRoutineId], 'a rotina de teste deve existir no JSON')
  assertEqual(routines[testRoutineId].cron, '0 14 * * *', 'a expressão cron para 14 horas deve ser parsed corretamente como 0 14 * * *')
  assertEqual(routines[testRoutineId].promptText, 'consulte o clima')

  // 3. Valida se gerou o arquivo de crontab correto
  const CRONTAB_FILE = path.join(workdir, 'crontab')
  const crontabContent = await fs.readFile(CRONTAB_FILE, 'utf8')
  assert(crontabContent.includes('0 14 * * *'), 'crontab deve conter o agendamento de 14 horas')
  assert(crontabContent.includes('http://host.docker.internal:3120/api/message'), 'crontab deve chamar a API REST')

  // 4. Testa a listagem das rotinas
  const listRes = await handler.run({ action: 'list' }, { taskId: 'test' })
  assert(listRes.includes(testRoutineId), 'a listagem deve conter a rotina de teste')

  // 5. Remove a rotina cadastrada para limpar o ambiente de testes
  const removeRes = await handler.run({
    action: 'remove',
    routineId: testRoutineId
  }, { taskId: 'test' })

  assert(removeRes.includes('sucesso') || removeRes.includes('localmente'), 'deve retornar mensagem de sucesso na remoção')

  // 6. Confere se foi removido do JSON
  const finalContent = await fs.readFile(ROUTINES_FILE, 'utf8')
  const finalRoutines = JSON.parse(finalContent)
  assert(!finalRoutines[testRoutineId], 'a rotina de teste deve ter sido removida do JSON')

  // Limpeza final do workspace de teste do cron
  await fs.rm(workdir, { recursive: true, force: true }).catch(() => {})
})

await test('actionRegistry carrega e executa skill declarada em subpasta (via skill.md)', async () => {
  const tempSkillDirPath = path.resolve('src/skills/skill_temp_folder')
  const tempMdPath = path.join(tempSkillDirPath, 'skill.md')

  // 1. Cria a subpasta
  await fs.mkdir(tempSkillDirPath, { recursive: true })

  const mdContent = `# skill_temp_folder

Prints a dynamic welcome message for folder testing.

## Parameters
- \`name\` (string, required): The name of the user

## Code
\`\`\`javascript
return \`Bem-vindo à skill em pasta, \${name}!\`;
\`\`\`
`

  // 2. Escreve o arquivo skill.md na subpasta
  await fs.writeFile(tempMdPath, mdContent, 'utf8')

  try {
    // 3. Recarrega as skills
    await actionRegistry.loadSkills()

    // 4. Confere se carregou
    const handler = actionRegistry.handlers.get('skill_temp_folder')
    assert(!!handler, 'deve encontrar o handler de skill_temp_folder')
    assertEqual(handler.constructor.description, 'Prints a dynamic welcome message for folder testing.')
    assertEqual(handler.constructor.params.name.required, true)

    // 5. Executa
    const res = await handler.run({ name: 'Maria' }, { taskId: 'test-folder' })
    assertEqual(res, 'Bem-vindo à skill em pasta, Maria!')
  } finally {
    // 6. Limpeza completa dos arquivos temporários
    await fs.unlink(tempMdPath).catch(() => {})
    await fs.rmdir(tempSkillDirPath).catch(() => {})
  }
})

await test('skill_browser executa automação de navegador com Playwright (auto-wait)', async () => {
  // Recarrega as skills para incluir a nova skill_browser
  await actionRegistry.loadSkills()

  const handler = actionRegistry.handlers.get('skill_browser')
  assert(!!handler, 'deve encontrar o handler de skill_browser')
  assertEqual(handler.constructor.description, "Navigates to a website and performs sequence of actions (click, fill, wait, text extraction, screenshots, evaluate) leveraging Playwright's native auto-waiting mechanisms.")
  assertEqual(handler.constructor.params.url.required, false)
  assertEqual(handler.constructor.params.actions.required, true)

  const taskId = 'test-browser-session-' + Date.now()

  // Fase 1: Navega para o site e salva o estado da sessão
  const phase1Actions = [
    { type: 'getText', selector: 'h1' }
  ]
  const res1 = await handler.run({
    url: 'https://example.com',
    actions: JSON.stringify(phase1Actions)
  }, { taskId })

  assert(res1.includes('Example Domain'), 'o resultado da fase 1 deve conter o texto extraído do h1 ("Example Domain")')

  // Fase 2: Invoca a skill SEM o parâmetro "url". Ela deve restaurar a URL e a sessão salvas!
  const phase2Actions = [
    { type: 'evaluate', script: 'document.title' }
  ]
  const res2 = await handler.run({
    actions: JSON.stringify(phase2Actions)
  }, { taskId })

  assert(res2.includes('Example Domain'), 'o resultado da fase 2 deve ter restaurado a sessão anterior e retornado o título do site ("Example Domain")')
  assert(res2.includes('[Session] Persisted browser storageState'), 'deve indicar o salvamento persistido da sessão')

  // Limpa arquivos de sessão e prints temporários do teste no workspace
  const fs = await import('fs/promises')
  const path = await import('path')
  const browserTestWorkspace = path.resolve('workspace', taskId)
  await fs.rm(browserTestWorkspace, { recursive: true, force: true }).catch(() => {})
})

await test('skill_telegram_send envia arquivos e imagens para o Telegram com sucesso', async () => {
  const { default: TelegramBot } = await import('node-telegram-bot-api');

  // Guardar métodos originais do protótipo
  const originalSendPhoto = TelegramBot.prototype.sendPhoto;
  const originalSendDocument = TelegramBot.prototype.sendDocument;

  // Interceptar chamadas
  const sentFiles = [];
  TelegramBot.prototype.sendPhoto = async function(chatId, filePath, options) {
    sentFiles.push({ type: 'photo', chatId, filePath, options });
    return { message_id: 999 };
  };
  TelegramBot.prototype.sendDocument = async function(chatId, filePath, options) {
    sentFiles.push({ type: 'document', chatId, filePath, options });
    return { message_id: 1000 };
  };

  // Definir token de teste se não houver
  const oldToken = process.env.TELEGRAM_TOKEN;
  process.env.TELEGRAM_TOKEN = process.env.TELEGRAM_TOKEN || '123456:FakeToken';

  try {
    // Recarrega as skills para incluir skill_telegram_send
    await actionRegistry.loadSkills();

    const handler = actionRegistry.handlers.get('skill_telegram_send');
    assert(!!handler, 'deve encontrar o handler de skill_telegram_send');

    // 1. Testa envio de imagem (deve usar sendPhoto)
    const testImagePath = path.resolve('test/temp_test_image.png');
    await fs.writeFile(testImagePath, 'fake image content', 'utf8');

    const res1 = await handler.run({
      filePath: 'test/temp_test_image.png',
      chatId: '456789',
      caption: 'Imagem de teste'
    }, { taskId: 'test-tg' });

    assert(res1.includes('sent successfully'), 'deve indicar que o arquivo foi enviado');
    assertEqual(sentFiles.length, 1);
    assertEqual(sentFiles[0].type, 'photo');
    assertEqual(sentFiles[0].chatId, '456789');
    assertEqual(sentFiles[0].options.caption, 'Imagem de teste');

    // 2. Testa envio de documento genérico (deve usar sendDocument e usar context.userId como fallback)
    const testDocPath = path.resolve('test/temp_test_doc.txt');
    await fs.writeFile(testDocPath, 'fake text content', 'utf8');

    const res2 = await handler.run({
      filePath: 'test/temp_test_doc.txt',
      caption: 'Documento de teste'
    }, { taskId: 'test-tg', userId: 'active-chat-999' });

    assert(res2.includes('sent successfully'), 'deve indicar que o arquivo foi enviado');
    assertEqual(sentFiles.length, 2);
    assertEqual(sentFiles[1].type, 'document');
    assertEqual(sentFiles[1].chatId, 'active-chat-999'); // Fallback para context.userId
    assertEqual(sentFiles[1].options.caption, 'Documento de teste');

    // Limpar arquivos temporários
    await fs.unlink(testImagePath).catch(() => {});
    await fs.unlink(testDocPath).catch(() => {});

  } finally {
    // Restaurar métodos originais
    TelegramBot.prototype.sendPhoto = originalSendPhoto;
    TelegramBot.prototype.sendDocument = originalSendDocument;
    
    if (oldToken === undefined) {
      delete process.env.TELEGRAM_TOKEN;
    } else {
      process.env.TELEGRAM_TOKEN = oldToken;
    }
  }
})

await test('skill_download baixa arquivos de links públicos com sucesso e bloqueia rede local', async () => {
  // Recarrega as skills para incluir skill_download
  await actionRegistry.loadSkills();

  const handler = actionRegistry.handlers.get('skill_download');
  assert(!!handler, 'deve encontrar o handler de skill_download');
  assertEqual(handler.constructor.params.url.required, true);

  // 1. Testa download bem-sucedido de exemplo público
  const testFileName = 'download_test_example.html';
  const targetPath = path.resolve('workspace', 'test-dl', testFileName);

  // Limpa se já existir
  await fs.unlink(targetPath).catch(() => {});

  const res = await handler.run({
    url: 'https://example.com/',
    fileName: testFileName
  }, { taskId: 'test-dl' });

  assert(res.includes('Successfully downloaded'), 'deve indicar sucesso no download');
  
  // Confirma se o arquivo existe e tem conteúdo
  const stat = await fs.stat(targetPath);
  assert(stat.size > 0, 'o arquivo baixado deve possuir tamanho maior que zero');

  // Limpa o diretório do teste gerado
  await fs.rm(path.resolve('workspace', 'test-dl'), { recursive: true, force: true }).catch(() => {});

  // 2. Testa bloqueio de segurança para rede local
  try {
    await handler.run({
      url: 'http://127.0.0.1/file.zip'
    }, { taskId: 'test-dl' });
    assert(false, 'deveria ter falhado para IP local 127.0.0.1');
  } catch (err) {
    assert(err.message.includes('blocked for security reasons'), 'deve indicar bloqueio por segurança');
  }
})

await test('skill_git executa operações Git (clone, status, pull) com sucesso', async () => {
  // Recarrega as skills para incluir skill_git
  await actionRegistry.loadSkills();

  const handler = actionRegistry.handlers.get('skill_git');
  assert(!!handler, 'deve encontrar o handler de skill_git');
  assertEqual(handler.constructor.params.action.required, true);

  const dummyRepoPath = path.resolve('test/dummy_git');
  const cloneDestFolder = 'test-git-clone';

  // 1. Cria um repositório git local temporário para servir de origem
  const shellHandler = actionRegistry.handlers.get('shell');
  await fs.mkdir(dummyRepoPath, { recursive: true });
  
  // Inicializa repositório de teste local
  await shellHandler.run({ command: `git init "${dummyRepoPath}"` }, { taskId: null });
  await shellHandler.run({ command: 'git config user.email "test@example.com"', cwd: 'test/dummy_git' }, { taskId: null });
  await shellHandler.run({ command: 'git config user.name "Test"', cwd: 'test/dummy_git' }, { taskId: null });
  
  const testFilePath = path.join(dummyRepoPath, 'test.txt');
  await fs.writeFile(testFilePath, 'Hello Git', 'utf8');
  await shellHandler.run({ command: 'git add test.txt', cwd: 'test/dummy_git' }, { taskId: null });
  await shellHandler.run({ command: 'git commit -m "initial commit"', cwd: 'test/dummy_git' }, { taskId: null });

  try {
    // 2. Executa a ação "clone" da skill_git
    const taskId = 'test-git-task-' + Date.now();
    
    const resClone = await handler.run({
      action: 'clone',
      repoUrl: `file://${dummyRepoPath.replace(/\\/g, '/')}`,
      destFolder: cloneDestFolder
    }, { taskId });

    assert(resClone.includes('clone') || resClone.includes('Cloning'), 'deve indicar sucesso no clone');

    // Validação física: o arquivo clonado deve existir no disco no local correto
    const clonedFilePath = path.resolve('workspace', taskId, cloneDestFolder, 'test.txt');
    const statCloned = await fs.stat(clonedFilePath);
    assert(statCloned.size > 0, 'o arquivo clonado deve existir fisicamente e possuir tamanho maior que zero');

    // 3. Executa a ação "status" na subpasta clonada
    const resStatus = await handler.run({
      action: 'status',
      destFolder: cloneDestFolder
    }, { taskId });

    assert(resStatus.includes('git status'), 'deve retornar a saída correta do git status');

    // 4. Executa a ação "pull" na subpasta clonada
    const resPull = await handler.run({
      action: 'pull',
      destFolder: cloneDestFolder
    }, { taskId });

    assert(resPull.includes('git pull'), 'deve retornar sucesso do git pull');

    // Limpa arquivos do clone gerados no workspace do teste
    const taskIdWorkspace = path.resolve('workspace', taskId);
    await fs.rm(taskIdWorkspace, { recursive: true, force: true }).catch(() => {});

  } finally {
    // Limpa o repositório de origem temporário
    await fs.rm(dummyRepoPath, { recursive: true, force: true }).catch(() => {});
  }
})

await test('skill_task_manager executa list, search, files e copy com sucesso entre workspaces', async () => {
  // Recarrega as skills para incluir skill_task_manager
  await actionRegistry.loadSkills();

  const handler = actionRegistry.handlers.get('skill_task_manager');
  assert(!!handler, 'deve encontrar o handler de skill_task_manager');
  assertEqual(handler.constructor.params.action.required, true);

  const pastTaskId = 'test-past-task-' + Date.now();
  const activeTaskId = 'test-active-task-' + Date.now();
  
  const pastWorkdir = path.resolve('workspace', pastTaskId);
  await fs.mkdir(pastWorkdir, { recursive: true });

  // 1. Cria arquivos na tarefa passada: sobre_a_tarefa.md e um arquivo de dados
  const pastDesc = `# O.R.E.I.A.S.E.C.A — Detalhes da Tarefa\n\n## Objetivo Principal\n> Desenvolver banco de dados fake para testes do gerenciador\n\n- **Criado em:** 28/05/2026 22:00:00\n- **Status Inicial:** done\n`;
  await fs.writeFile(path.join(pastWorkdir, 'sobre_a_tarefa.md'), pastDesc, 'utf8');
  await fs.writeFile(path.join(pastWorkdir, 'data.txt'), 'conteudo super importante', 'utf8');

  try {
    // 2. Executa a ação "list"
    const resList = await handler.run({ action: 'list' }, { taskId: activeTaskId });
    assert(resList.includes(pastTaskId), 'list deve conter o ID da tarefa passada');
    assert(resList.includes('Desenvolver banco de dados fake'), 'list deve conter o objetivo da tarefa passada');

    // 3. Executa a ação "search" com query bem-sucedida
    const resSearch = await handler.run({ action: 'search', query: 'banco' }, { taskId: activeTaskId });
    assert(resSearch.includes(pastTaskId), 'search deve encontrar a tarefa pelo termo "banco"');

    // 4. Executa a ação "search" sem correspondências
    const resSearchEmpty = await handler.run({ action: 'search', query: 'inexistente' }, { taskId: activeTaskId });
    assert(resSearchEmpty.includes('Nenhuma tarefa correspondente'), 'search deve retornar aviso se nada for encontrado');

    // 5. Executa a ação "files"
    const resFiles = await handler.run({ action: 'files', taskId: pastTaskId }, { taskId: activeTaskId });
    assert(resFiles.includes('sobre_a_tarefa.md'), 'files deve conter o arquivo de descrição');
    assert(resFiles.includes('data.txt'), 'files deve conter o arquivo de dados');

    // 6. Executa a ação "copy" para o workspace ativo
    const resCopy = await handler.run({
      action: 'copy',
      taskId: pastTaskId,
      filePath: 'data.txt',
      destPath: 'importado/dados_copiados.txt'
    }, { taskId: activeTaskId });

    assert(resCopy.includes('Successfully copied'), 'copy deve indicar sucesso na cópia');

    // Validação física da cópia no disco
    const copiedFilePath = path.resolve('workspace', activeTaskId, 'importado', 'dados_copiados.txt');
    const statCopied = await fs.stat(copiedFilePath);
    assert(statCopied.size > 0, 'o arquivo copiado deve existir fisicamente no workspace ativo');
    
    const copiedContent = await fs.readFile(copiedFilePath, 'utf8');
    assertEqual(copiedContent, 'conteudo super importante');

  } finally {
    // Limpeza dos diretórios de teste gerados
    await fs.rm(path.resolve('workspace', pastTaskId), { recursive: true, force: true }).catch(() => {});
    await fs.rm(path.resolve('workspace', activeTaskId), { recursive: true, force: true }).catch(() => {});
  }
})

await test('skill_telegram_send resolve arquivos de forma relativa ao workspace ativo no teste', async () => {
  const { default: TelegramBot } = await import('node-telegram-bot-api');
  const originalSendDocument = TelegramBot.prototype.sendDocument;
  const sentFiles = [];

  TelegramBot.prototype.sendDocument = async function(chatId, filePath, options) {
    sentFiles.push({ chatId, filePath, options });
    return { message_id: 1001 };
  };

  const oldToken = process.env.TELEGRAM_TOKEN;
  process.env.TELEGRAM_TOKEN = process.env.TELEGRAM_TOKEN || '123456:FakeToken';

  const taskId = 'test-tg-workspace-' + Date.now();
  const workdir = path.resolve('workspace', taskId);
  await fs.mkdir(workdir, { recursive: true });

  const testFile = path.join(workdir, 'documento_tarefa.txt');
  await fs.writeFile(testFile, 'arquivo de texto da tarefa', 'utf8');

  try {
    const handler = actionRegistry.handlers.get('skill_telegram_send');
    
    // Executa passando caminho relativo, sem diretório especial
    const res = await handler.run({
      filePath: 'documento_tarefa.txt',
      chatId: '777'
    }, { taskId });

    assert(res.includes('sent successfully'), 'deve indicar sucesso no envio');
    assertEqual(sentFiles.length, 1);
    assertEqual(sentFiles[0].chatId, '777');
    
    // O caminho resolvido enviado ao bot do telegram deve ser o caminho físico absoluto dentro do workspace!
    assertEqual(path.resolve(sentFiles[0].filePath), testFile);

  } finally {
    // Restaurar ambiente e fazer limpeza
    TelegramBot.prototype.sendDocument = originalSendDocument;
    if (oldToken === undefined) {
      delete process.env.TELEGRAM_TOKEN;
    } else {
      process.env.TELEGRAM_TOKEN = oldToken;
    }
    await fs.rm(workdir, { recursive: true, force: true }).catch(() => {});
  }
})

// ─── Resultado final ──────────────────────────────────────────────────────────
console.log(`\n${c.bold}${'─'.repeat(40)}${c.reset}`)
console.log(`  ${c.green}${c.bold}${passed} passaram${c.reset}  ${failed > 0 ? c.red + c.bold : c.gray}${failed} falharam${c.reset}`)

if (failed > 0) {
  process.exit(1)
} else {
  setTimeout(() => process.exit(0), 200)
}
