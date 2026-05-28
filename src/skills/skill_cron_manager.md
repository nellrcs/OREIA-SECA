# skill_cron_manager

CRITICAL: This skill MUST be used for ALL operations (adding, listing, removing, checking) regarding cron routines, tasks scheduling, recurring routines, or "rotinas do cron". NEVER use the raw "shell" command to list, check, add, or remove cron schedules, as all scheduling in this application is managed isolately inside a Docker container via this skill.

## Parameters
- `action` (string, required): The action to perform, either "add", "remove", or "list".
- `routineId` (string, optional): A unique identifier for the routine (required for "add" and "remove", e.g., "weather_daily").
- `promptText` (string, optional): The prompt command/text to send to the AI when the schedule fires (required for "add", e.g., "consulte o clima").
- `cronExpression` (string, optional): A standard 5-field cron expression (e.g., "0 14 * * *").
- `textSchedule` (string, optional): A natural language schedule description (e.g., "todos os dias as 14 horas", "a cada 5 minutos").

## Code
```javascript
const fs = await import('fs/promises');
const path = await import('path');
const { execSync } = await import('child_process');

const ROUTINES_FILE = path.resolve('tasks/cron_routines.json');
const CRONTAB_FILE = path.resolve('tasks/crontab');

// Helper to parse natural language to cron
function parseTextToCron(text) {
  const clean = text.toLowerCase().trim();
  
  // Ex: "todos os dias as 14 horas", "diariamente as 14:00"
  let match = clean.match(/(?:todos os dias|diariamente)\s+as\s+(\d+)(?::(\d+))?/);
  if (match) {
    const hour = parseInt(match[1]);
    const min = match[2] ? parseInt(match[2]) : 0;
    return `${min} ${hour} * * *`;
  }

  // Ex: "a cada 5 minutos", "de 5 em 5 minutos"
  match = clean.match(/(?:a cada|de\s+\d+\s+em)\s+(\d+)\s+minuto/);
  if (match) {
    const mins = parseInt(match[1]);
    return `*/${mins} * * * *`;
  }

  // Ex: "a cada 2 horas", "de 2 em 2 horas"
  match = clean.match(/(?:a cada|de\s+\d+\s+em)\s+(\d+)\s+hora/);
  if (match) {
    const hours = parseInt(match[1]);
    return `0 */${hours} * * *`;
  }

  throw new Error(`Could not parse natural language schedule: "${text}". Please provide a standard cron expression.`);
}

// 1. Ensure tasks folder exists
await fs.mkdir(path.resolve('tasks'), { recursive: true });

// 2. Load existing routines
let routines = {};
try {
  const content = await fs.readFile(ROUTINES_FILE, 'utf8');
  routines = JSON.parse(content);
} catch {}

const act = action.toLowerCase();

if (act === 'list') {
  const list = Object.entries(routines).map(([id, r]) => {
    return `- **${id}**: "${r.promptText}" [Schedule: \`${r.cron}\`]`;
  });
  if (!list.length) return 'Nenhum agendamento ativo no momento.';
  return `Rotinas agendadas ativas:\n${list.join('\n')}`;
}

if (act === 'remove') {
  if (!routineId) throw new Error('Parameter "routineId" is required for action "remove".');
  if (!routines[routineId]) {
    return `A rotina "${routineId}" não foi encontrada.`;
  }
  delete routines[routineId];
  await fs.writeFile(ROUTINES_FILE, JSON.stringify(routines, null, 2));
}

if (act === 'add') {
  if (!routineId) throw new Error('Parameter "routineId" is required for action "add".');
  if (!promptText) throw new Error('Parameter "promptText" is required for action "add".');
  
  let cron = cronExpression;
  if (!cron && textSchedule) {
    cron = parseTextToCron(textSchedule);
  }
  if (!cron) {
    throw new Error('Either "cronExpression" or "textSchedule" is required for action "add".');
  }

  routines[routineId] = {
    promptText,
    cron,
    createdAt: new Date().toISOString()
  };
  await fs.writeFile(ROUTINES_FILE, JSON.stringify(routines, null, 2));
}

// 3. Rebuild crontab content
// We construct curl commands calling the local REST API message endpoint
const crontabLines = [];
crontabLines.push('# crontab para oreiaseca-cron');
crontabLines.push('# Gerado automaticamente em ' + new Date().toISOString());

for (const [id, r] of Object.entries(routines)) {
  const payload = JSON.stringify({ text: r.promptText, userId: `cron-${id}` });
  // Escape single quotes in JSON string for bash execution
  const escapedPayload = payload.replace(/'/g, "'\\''");
  
  // Note: we target host.docker.internal which is resolveable inside Docker containers
  const curlCmd = `curl -s -X POST -H "Content-Type: application/json" -d '${escapedPayload}' http://host.docker.internal:3120/api/message`;
  crontabLines.push(`${r.cron} ${curlCmd}`);
}
// Cron requires an empty line at the end of the crontab file to parse correctly
crontabLines.push('');

await fs.writeFile(CRONTAB_FILE, crontabLines.join('\n'));

// 4. Synchronize with Docker Container
try {
  // Test if Docker is available
  execSync('docker --version', { stdio: 'ignore' });
} catch (err) {
  return `Rotina salva localmente, mas Docker não está em execução ou não foi encontrado no PATH. Certifique-se de que o Docker esteja ativo para que o agendamento funcione.`;
}

try {
  // Check if oreiaseca-cron container exists and remove it to rebuild cleanly
  try {
    execSync('docker rm -f oreiaseca-cron', { stdio: 'ignore' });
  } catch {}

  const dockerCmd = `docker run -d --name oreiaseca-cron ` +
    `-v "${CRONTAB_FILE}:/etc/crontabs/root" ` +
    `--add-host host.docker.internal:host-gateway ` +
    `alpine sh -c "apk add --no-cache curl && crond -f -l 2"`;
  
  execSync(dockerCmd, { stdio: 'ignore' });

  if (act === 'add') {
    return `Rotina "${routineId}" agendada com sucesso! Container Docker de cron sincronizado e em execução.`;
  } else {
    return `Rotina "${routineId}" removida com sucesso. Container Docker de cron atualizado.`;
  }
} catch (err) {
  return `Erro ao gerenciar container Docker de cron: ${err.message}. A rotina foi salva localmente mas pode não disparar até que o container Docker seja restabelecido.`;
}
```
