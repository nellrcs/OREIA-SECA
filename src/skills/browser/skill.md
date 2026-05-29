# skill_browser

Navigates to a website and performs sequence of actions (click, fill, wait, text extraction, screenshots, evaluate) leveraging Playwright's native auto-waiting mechanisms.

## Parameters
- `url` (string, optional): The initial URL to navigate to (e.g., https://example.com). If omitted and a previous step in the same task has run, it will restore the session at the last visited page.
- `actions` (string, required): A JSON array of actions to execute sequentially. E.g. `[{"type": "click", "selector": "button"}]`.
- `browser` (string, optional): The browser type: "chromium" (default), "firefox", or "webkit".
- `headless` (string, optional): "true" (default) or "false" to view browser execution UI.
- `timeout` (number, optional): Timeout in milliseconds for each auto-waiting action. Defaults to 15000.

## Code
```javascript
const { chromium, firefox, webkit } = await import('playwright');
const fs = await import('fs/promises');
const path = await import('path');

const parsedActions = JSON.parse(actions);
if (!Array.isArray(parsedActions)) {
  throw new Error('skill_browser: "actions" parameter must be a JSON array');
}

// 1. Resolve browser and launch options
const browserType = (browser || 'chromium').toLowerCase();
const isHeadless = headless !== 'false';
const defaultTimeout = Number(timeout) || 15000;

let selectedBrowser;
if (browserType === 'firefox') {
  selectedBrowser = firefox;
} else if (browserType === 'webkit') {
  selectedBrowser = webkit;
} else {
  selectedBrowser = chromium;
}

const browserInstance = await selectedBrowser.launch({
  headless: isHeadless
});

let statePath = null;
let metaPath = null;
const contextOptions = {};

const workdir = (context && context.taskId)
  ? path.resolve('workspace', context.taskId)
  : path.resolve('workspace');

// Ensure the workspace folder exists
await fs.mkdir(workdir, { recursive: true });

if (context && context.taskId) {
  statePath = path.join(workdir, 'browser_state.json');
  metaPath = path.join(workdir, 'browser_meta.json');
  
  try {
    await fs.access(statePath);
    contextOptions.storageState = statePath;
  } catch (err) {
    // Session state file doesn't exist yet
  }
}

const browserContext = await browserInstance.newContext(contextOptions);
const page = await browserContext.newPage();
page.setDefaultTimeout(defaultTimeout);

const results = [];

try {
  // 2. Perform initial navigation
  let initialUrl = url;
  if (!initialUrl && metaPath) {
    try {
      const metaContent = await fs.readFile(metaPath, 'utf-8');
      const meta = JSON.parse(metaContent);
      initialUrl = meta.lastUrl;
    } catch (e) {
      // Previous session URL is not available
    }
  }

  if (initialUrl) {
    results.push(`Navigating to: ${initialUrl}`);
    await page.goto(initialUrl, { waitUntil: 'load' });
    results.push(`Successfully loaded: ${initialUrl}`);
  } else {
    results.push('No URL provided and no previous session URL found. Starting on a blank page.');
  }

  // 3. Process actions
  for (let idx = 0; idx < parsedActions.length; idx++) {
    const act = parsedActions[idx];
    const stepLabel = `Step ${idx + 1} (${act.type})`;

    if (act.type === 'navigate') {
      if (!act.url) throw new Error(`${stepLabel}: missing "url" param`);
      await page.goto(act.url, { waitUntil: 'load' });
      results.push(`${stepLabel}: Navigated to ${act.url}`);
    }
    
    else if (act.type === 'click') {
      if (!act.selector) throw new Error(`${stepLabel}: missing "selector" param`);
      // Playwright auto-waits for element to be attached, visible, stable, and ready to receive clicks
      await page.click(act.selector);
      results.push(`${stepLabel}: Clicked element "${act.selector}"`);
    }
    
    else if (act.type === 'fill') {
      if (!act.selector) throw new Error(`${stepLabel}: missing "selector" param`);
      if (act.value === undefined) throw new Error(`${stepLabel}: missing "value" param`);
      // Playwright auto-waits for element to be editable
      await page.fill(act.selector, String(act.value));
      results.push(`${stepLabel}: Filled element "${act.selector}" with value`);
    }
    
    else if (act.type === 'press') {
      if (!act.selector) throw new Error(`${stepLabel}: missing "selector" param`);
      if (!act.key) throw new Error(`${stepLabel}: missing "key" param (e.g. "Enter")`);
      await page.press(act.selector, act.key);
      results.push(`${stepLabel}: Pressed "${act.key}" on element "${act.selector}"`);
    }
    
    else if (act.type === 'select') {
      if (!act.selector) throw new Error(`${stepLabel}: missing "selector" param`);
      if (act.value === undefined) throw new Error(`${stepLabel}: missing "value" param`);
      await page.selectOption(act.selector, String(act.value));
      results.push(`${stepLabel}: Selected option "${act.value}" on "${act.selector}"`);
    }
    
    else if (act.type === 'waitFor') {
      if (!act.selector) throw new Error(`${stepLabel}: missing "selector" param`);
      const state = act.state || 'visible'; // 'attached', 'detached', 'visible', 'hidden'
      await page.waitForSelector(act.selector, { state });
      results.push(`${stepLabel}: Element "${act.selector}" is now ${state}`);
    }
    
    else if (act.type === 'getText') {
      if (!act.selector) throw new Error(`${stepLabel}: missing "selector" param`);
      const text = await page.locator(act.selector).first().innerText();
      results.push(`${stepLabel}: Text inside "${act.selector}": "${text}"`);
    }
    
    else if (act.type === 'getHTML') {
      if (!act.selector) throw new Error(`${stepLabel}: missing "selector" param`);
      const html = await page.locator(act.selector).first().innerHTML();
      results.push(`${stepLabel}: HTML of "${act.selector}": "${html}"`);
    }
    
    else if (act.type === 'screenshot') {
      const name = act.name || `screenshot_${Date.now()}.png`;
      // Ensure we place screenshots inside the workspace folder safely
      const imgPath = path.join(workdir, name);
      
      await page.screenshot({ path: imgPath, fullPage: !!act.fullPage });
      results.push(`${stepLabel}: Screenshot saved to workspace/${context.taskId || 'default'}/${name}`);
    }
    
    else if (act.type === 'evaluate') {
      if (!act.script) throw new Error(`${stepLabel}: missing "script" param`);
      const evalRes = await page.evaluate(act.script);
      results.push(`${stepLabel}: Script evaluated: ${JSON.stringify(evalRes)}`);
    }
    
    else {
      throw new Error(`skill_browser: Unknown action type "${act.type}" at step ${idx + 1}`);
    }
  }

  // 4. Save storageState and last URL for session persistence
  if (statePath && metaPath) {
    await browserContext.storageState({ path: statePath });
    const finalUrl = page.url();
    await fs.writeFile(metaPath, JSON.stringify({ lastUrl: finalUrl }, null, 2), 'utf-8');
    results.push(`[Session] Persisted browser storageState and current URL (${finalUrl}) for the active task context.`);
  }

  return `Browser automation execution summary:\n\n${results.join('\n')}`;

} finally {
  await browserInstance.close();
}
```
