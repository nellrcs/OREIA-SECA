# skill_telegram_send

Sends a local file (photo, document, log, etc.) to a specific Telegram Chat ID or defaults to the active task's session user chat.

## Parameters
- `filePath` (string, required): The relative path of the file to send (e.g. `tasks/screenshot.png` or `README.md`). Traversal via ".." is blocked.
- `chatId` (string, optional): The Telegram Chat ID to send the file to. If omitted, it will automatically fall back to the active user's chat ID from the task execution context.
- `caption` (string, optional): An optional text caption/description to send along with the file.

## Code
```javascript
const fs = await import('fs/promises');
const path = await import('path');
const TelegramBotModule = await import('node-telegram-bot-api');
const TelegramBot = TelegramBotModule.default || TelegramBotModule;

// 1. Sanity Checks & Security
if (!filePath) {
  throw new Error('TelegramSend: parameter "filePath" is required');
}
if (filePath.includes('..')) {
  throw new Error('TelegramSend: path traversal blocked');
}

const targetChatId = chatId || (context && context.userId);
if (!targetChatId) {
  throw new Error('TelegramSend: Target "chatId" was not provided and no active userId was found in execution context.');
}

const token = process.env.TELEGRAM_TOKEN;
if (!token) {
  throw new Error('TelegramSend: TELEGRAM_TOKEN environment variable not set. Please set it in your .env configuration.');
}

// 2. Resolve and verify file existence
const isSpecialDir = filePath.startsWith('tasks/') || filePath === 'tasks' || 
                     filePath.startsWith('workspace/') || filePath === 'workspace' ||
                     filePath.startsWith('test/') || filePath === 'test';

const workdir = isSpecialDir
  ? path.resolve()
  : (context && context.taskId ? path.resolve('workspace', context.taskId) : path.resolve('workspace'));

const resolvedPath = path.resolve(workdir, filePath);

try {
  await fs.access(resolvedPath);
} catch (err) {
  throw new Error(`TelegramSend: File not found or not readable: ${filePath}`);
}

// 3. Initialize bot client in direct api mode (no polling)
const bot = new TelegramBot(token, { polling: false });

// 4. Determine media type and send
const extension = path.extname(filePath).toLowerCase();
const imageExtensions = ['.png', '.jpg', '.jpeg', '.webp', '.gif'];
const isPhoto = imageExtensions.includes(extension);

const options = {};
if (caption) {
  options.caption = caption;
}

try {
  if (isPhoto) {
    await bot.sendPhoto(targetChatId, resolvedPath, options);
  } else {
    await bot.sendDocument(targetChatId, resolvedPath, options);
  }
  return `File "${filePath}" sent successfully to Telegram chat ${targetChatId}.`;
} catch (err) {
  throw new Error(`TelegramSend failed: ${err.message}`);
}
```
