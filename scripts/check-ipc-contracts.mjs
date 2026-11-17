import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

function walk(root) {
  return readdirSync(root).flatMap((name) => {
    const child = join(root, name);
    return statSync(child).isDirectory() ? walk(child) : [child];
  });
}

function duplicates(values) {
  return [...new Set(values.filter((value, index) => values.indexOf(value) !== index))];
}

function fail(message, values = []) {
  const suffix = values.length ? `: ${values.sort().join(', ')}` : '';
  throw new Error(`${message}${suffix}`);
}

const handlerSource = readFileSync('src-tauri/src/invoke_handler.rs', 'utf8');
const registeredCommands = [
  ...handlerSource.matchAll(/crate::(?:[A-Za-z0-9_]+::)*([A-Za-z0-9_]+),/g),
].map((match) => match[1]);
if (!registeredCommands.includes('hardware_safety_status')) {
  fail('The central hardware safety command is not registered');
}
const duplicateCommands = duplicates(registeredCommands);
if (duplicateCommands.length) fail('Duplicate Tauri command registrations', duplicateCommands);
const invalidCommandNames = registeredCommands.filter(
  (command) => !/^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$/.test(command)
);
if (invalidCommandNames.length) fail('Invalid Tauri command names', invalidCommandNames);

const rustPaths = walk('src-tauri/src').filter((path) => path.endsWith('.rs'));
const annotatedCommands = [];
for (const path of rustPaths) {
  const lines = readFileSync(path, 'utf8').split('\n');
  for (let index = 0; index < lines.length; index += 1) {
    if (!/^#\[(?:tauri::)?command\]/.test(lines[index].trim())) continue;
    for (let cursor = index + 1; cursor < Math.min(index + 12, lines.length); cursor += 1) {
      const match = lines[cursor].match(/(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)/);
      if (match) {
        annotatedCommands.push(match[1]);
        break;
      }
    }
  }
}
const unregisteredAnnotations = [...new Set(annotatedCommands)].filter(
  (command) => !registeredCommands.includes(command)
);
if (unregisteredAnnotations.length) {
  fail('Tauri commands exist but are not registered', unregisteredAnnotations);
}

const contractSource = readFileSync('src/Platform/IPC/Commands.ts', 'utf8');
const commandStart = contractSource.indexOf('export interface IpcCommandMap');
const eventStart = contractSource.indexOf('export interface IpcEventMap');
if (commandStart < 0 || eventStart < 0) fail('IPC command or event map is missing');
const domainContractSource = walk('src/Platform/IPC/contracts')
  .filter((path) => path.endsWith('.ts'))
  .map((path) => readFileSync(path, 'utf8'))
  .join('\n');
const commandSource = `${contractSource.slice(commandStart, eventStart)}\n${domainContractSource}`;
const declaredCommands = [...commandSource.matchAll(/^  ([A-Za-z0-9_]+): IpcCommandSpec/gm)].map(
  (match) => match[1]
);
const duplicateDeclarations = duplicates(declaredCommands);
if (duplicateDeclarations.length)
  fail('Duplicate frontend command contracts', duplicateDeclarations);

const missingRegistrations = declaredCommands.filter(
  (command) => command !== 'open_devtools' && !registeredCommands.includes(command)
);
if (missingRegistrations.length) {
  fail('Frontend commands without a Tauri registration', missingRegistrations);
}
const undocumentedRegistrations = registeredCommands.filter(
  (command) => !declaredCommands.includes(command)
);
if (undocumentedRegistrations.length) {
  fail('Tauri registrations without a frontend contract', undocumentedRegistrations);
}

const declaredEvents = [
  ...`${contractSource.slice(eventStart)}\n${domainContractSource}`.matchAll(
    /^  ['"]?([A-Za-z0-9_-]+)['"]?: IpcEventSpec/gm
  ),
].map((match) => match[1]);
const duplicateEvents = duplicates(declaredEvents);
if (duplicateEvents.length) fail('Duplicate frontend event contracts', duplicateEvents);
const invalidEventNames = declaredEvents.filter(
  (event) => !/^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/.test(event)
);
if (invalidEventNames.length) fail('Invalid public event names', invalidEventNames);

const frontendSource = walk('src')
  .filter((path) => /\.(ts|tsx)$/.test(path))
  .map((path) => readFileSync(path, 'utf8'))
  .join('\n');
const subscribedEvents = [
  ...frontendSource.matchAll(/(?:subscribeEvent|listen)\(\s*['"]([A-Za-z0-9_-]+)['"]/g),
].map((match) => match[1]);
const undeclaredSubscriptions = [...new Set(subscribedEvents)].filter(
  (event) => !declaredEvents.includes(event)
);
if (undeclaredSubscriptions.length) {
  fail('Frontend event subscriptions without a contract', undeclaredSubscriptions);
}

const rustSource = rustPaths.map((path) => readFileSync(path, 'utf8')).join('\n');
const emittedEvents = [...rustSource.matchAll(/\.emit\(\s*['"]([A-Za-z0-9_-]+)['"]/g)].map(
  (match) => match[1]
);
const internalEvents = new Set(['mass-hotplug-internal']);
const undeclaredEmissions = [...new Set(emittedEvents)].filter(
  (event) => !internalEvents.has(event) && !declaredEvents.includes(event)
);
if (undeclaredEmissions.length) {
  fail('Backend events without a frontend contract or internal exemption', undeclaredEmissions);
}

console.log(
  `IPC contract gate passed: ${registeredCommands.length} commands and ` +
    `${declaredEvents.length} public events are covered.`
);
