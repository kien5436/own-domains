import { execFile } from 'node:child_process';
import { join } from 'node:path';

export type ServiceCommand = { command: string; args: string[]; ignoreFailure: boolean };

export function buildLaunchdActivationCommands(homeDirectory: string, userId: number, userName: string): ServiceCommand[] {
  return [
    bootout('system/dev.owndomains.dnsmasq'),
    bootout('system/dev.owndomains.caddy'),
    asUser(userName, bootout(`gui/${userId}/dev.owndomains.backend`)),
    bootstrap('system', '/Library/LaunchDaemons/dev.owndomains.dnsmasq.plist'),
    bootstrap('system', '/Library/LaunchDaemons/dev.owndomains.caddy.plist'),
    asUser(userName, bootstrap(`gui/${userId}`, join(homeDirectory, 'Library/LaunchAgents/dev.owndomains.backend.plist'))),
  ];
}

export function buildLaunchdDeactivationCommands(userId: number, userName: string): ServiceCommand[] {
  return [
    asUser(userName, bootout(`gui/${userId}/dev.owndomains.backend`)),
    bootout('system/dev.owndomains.caddy'),
    bootout('system/dev.owndomains.dnsmasq'),
  ];
}

export async function activateLaunchdServices(homeDirectory: string, userId: number, userName: string): Promise<void> {
  await executeCommands(buildLaunchdActivationCommands(homeDirectory, userId, userName), 0);
}

export async function deactivateLaunchdServices(userId: number, userName: string): Promise<void> {
  await executeCommands(buildLaunchdDeactivationCommands(userId, userName), 0);
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

function asUser(userName: string, serviceCommand: ServiceCommand): ServiceCommand {
  return { ...serviceCommand, command: '/usr/bin/sudo', args: ['-u', userName, '--', serviceCommand.command, ...serviceCommand.args] };
}
