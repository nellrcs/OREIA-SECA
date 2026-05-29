# skill_docker

Allows managing Docker container lifecycles safely, including verifying status, listing containers, stopping, starting, showing logs, removing existing containers, and running new configured instances.

Example XML to run a container:
<action name="skill_docker">
  <param name="action">run</param>
  <param name="containerName">my-python-server</param>
  <param name="image">python:3-alpine</param>
  <param name="ports">8787:8000</param>
  <param name="cmd">python -m http.server 8000</param>
</action>

## Parameters
- `action` (string, required): The Docker operation to perform ("version", "ps", "list", "remove", "run", "stop", "start", "logs").
- `containerName` (string, optional): O nome do container (required for "remove", "stop", "start", "logs", and "run").
- `image` (string, optional): Opcional image name for running a container.
- `ports` (string, optional): Port mapping/publishing (e.g. "8787:8000" or "80:80").
- `volumes` (string, optional): Directory/file volume mapping.
- `addHost` (string, optional): Optional extra host mappings.
- `cmd` (string, optional): The command to execute in the container.

## Code
```javascript
const path = await import('path');
const { pathToFileURL } = await import('url');

// Carregar dinamicamente o actionRegistry para acessar o handler do shell
const registryPath = path.resolve('src/actions/registry.js');
const { actionRegistry } = await import(pathToFileURL(registryPath).href);
const shellHandler = actionRegistry.handlers.get('shell');

const act = action.toLowerCase();

if (act === 'version') {
  try {
    const res = await shellHandler.run({ command: 'docker --version' }, context);
    return res;
  } catch (err) {
    throw new Error('Docker is not running or not installed.');
  }
}

if (act === 'ps' || act === 'list') {
  try {
    const res = await shellHandler.run({
      command: 'docker ps -a --format "table {{.ID}}\t{{.Names}}\t{{.Status}}\t{{.Image}}"'
    }, context);
    return res;
  } catch (err) {
    throw new Error(`Failed to list containers: ${err.message}`);
  }
}

if (act === 'stop') {
  if (!containerName) throw new Error('Parameter "containerName" is required for "stop" action.');
  try {
    await shellHandler.run({ command: `docker stop ${containerName}` }, context);
    return `Container "${containerName}" stopped successfully.`;
  } catch (err) {
    throw new Error(`Failed to stop container "${containerName}": ${err.message}`);
  }
}

if (act === 'start') {
  if (!containerName) throw new Error('Parameter "containerName" is required for "start" action.');
  try {
    await shellHandler.run({ command: `docker start ${containerName}` }, context);
    return `Container "${containerName}" started successfully.`;
  } catch (err) {
    throw new Error(`Failed to start container "${containerName}": ${err.message}`);
  }
}

if (act === 'logs') {
  if (!containerName) throw new Error('Parameter "containerName" is required for "logs" action.');
  try {
    const res = await shellHandler.run({ command: `docker logs --tail 50 ${containerName}` }, context);
    return res;
  } catch (err) {
    throw new Error(`Failed to read logs for container "${containerName}": ${err.message}`);
  }
}

if (act === 'remove') {
  if (!containerName) {
    throw new Error('Parameter "containerName" is required for "remove" action.');
  }
  try {
    await shellHandler.run({ command: `docker rm -f ${containerName}` }, context);
    return `Container "${containerName}" removed successfully (or did not exist).`;
  } catch (err) {
    throw new Error(`Failed to remove container "${containerName}": ${err.message}`);
  }
}

if (act === 'run') {
  if (!containerName) throw new Error('Parameter "containerName" is required for "run" action.');
  if (!image) throw new Error('Parameter "image" is required for "run" action.');
  
  let dockerCmd = `docker run -d --name ${containerName}`;
  
  if (ports) {
    dockerCmd += ` -p ${ports}`;
  }
  if (volumes) {
    dockerCmd += ` -v "${volumes}"`;
  }
  if (addHost) {
    dockerCmd += ` --add-host ${addHost}`;
  }
  
  dockerCmd += ` ${image}`;
  
  if (cmd) {
    dockerCmd += ` ${cmd}`;
  }
  
  try {
    await shellHandler.run({ command: dockerCmd }, context);
    return `Container "${containerName}" started successfully with image "${image}".`;
  } catch (err) {
    throw new Error(`Failed to start container "${containerName}": ${err.message}`);
  }
}

throw new Error(`Unknown action: "${action}"`);
```
