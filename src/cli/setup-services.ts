import { execFile } from 'node:child_process';
import { join } from 'node:path';

export type ServiceCommand = { command: string; args: string[]; ignoreFailure: boolean };

export function buildLaunchdActivationCommands(homeDirectory: string, userId: number): ServiceCommand[] {
  return [
    bootout('system/dev.owndomains.dnsmasq'),
    bootout('system/dev.owndomains.caddy'),
    bootout(`gui/${userId}/dev.owndomains.backend`),
    bootstrap('system', '/Library/LaunchDaemons/dev.owndomains.dnsmasq.plist'),
    bootstrap('system', '/Library/LaunchDaemons/dev.owndomains.caddy.plist'),
    bootstrap(`gui/${userId}`, join(homeDirectory, 'Library/LaunchAgents/dev.owndomains.backend.plist')),
  ];
}

export async function activateLaunchdServices(homeDirectory: string, userId: number): Promise<void> {
  await executeCommands(buildLaunchdActivationCommands(homeDirectory, userId), 0);
}

async function executeCommands(commands: ServiceCommand[], index: number): Promise<void> {
  const serviceCommand = commands[index];

  if (!serviceCommand) return;

  try {
    await execute(serviceCommand.command, serviceCommand.args);
  }
  catch (error) {
    if (!serviceCommand.ignoreFailure) throw error;
  }

  await executeCommands(commands, index + 1);
}

function execute(command: string, args: string[]): Promise<void> {
  return new Promise((resolve, reject) => {
    execFile(command, args, { timeout: 10_000 }, (error, stdout, stderr) => error ? reject(new Error(`${command} ${args.join(' ')} failed: ${stderr || stdout || error.message}`)) : resolve());
  });
}

function bootout(domain: string): ServiceCommand {
  return { command: '/bin/launchctl', args: ['bootout', domain], ignoreFailure: true };
}

function bootstrap(domain: string, path: string): ServiceCommand {
  return { command: '/bin/launchctl', args: ['bootstrap', domain, path], ignoreFailure: false };
}
