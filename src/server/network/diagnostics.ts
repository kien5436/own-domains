import { execFile } from 'node:child_process';
import { connect } from 'node:net';

import type { Diagnostic, DomainMapping } from '../../shared/types';

type NetworkState = { lanAddress: string; mappings: DomainMapping[] };
type Probe = (port: number) => Promise<boolean>;
type DnsEnvironment = { dnsConfiguration: string; port53Listeners: string; warpSettings: string };
type InspectDns = () => Promise<DnsEnvironment>;

export async function diagnoseNetwork(state: NetworkState, probe: Probe = probePort, inspectDns: InspectDns = inspectDnsEnvironment): Promise<Diagnostic[]> {
  const diagnostics: Diagnostic[] = [];

  diagnostics.push(state.lanAddress ? { code: 'lan-address', status: 'ok', message: `LAN address: ${state.lanAddress}` } : { code: 'lan-address', status: 'warning', message: 'No LAN address selected', remedy: 'Select the Mac LAN IPv4 address used by your phone.' });

  const upstreamDiagnostics = await Promise.all(state.mappings.filter((item) => item.enabled).map(async (mapping): Promise<Diagnostic> => {
    const reachable = await probe(mapping.upstreamPort);

    return reachable ? { code: `upstream:${mapping.hostname}`, status: 'ok', message: `${mapping.hostname} can reach port ${mapping.upstreamPort}` } : { code: `upstream:${mapping.hostname}`, status: 'error', message: `${mapping.hostname} cannot reach port ${mapping.upstreamPort}`, remedy: `Start the local app on 127.0.0.1:${mapping.upstreamPort}.` };
  }));

  diagnostics.push(...upstreamDiagnostics);

  try {
    diagnostics.push(...diagnoseDnsEnvironment(state.lanAddress, await inspectDns()));
  }
  catch (error) {
    diagnostics.push({ code: 'dns-inspection', status: 'warning', message: 'Unable to inspect local DNS services', remedy: error instanceof Error ? error.message : 'Run Own Domains doctor from a terminal.' });
  }

  return diagnostics;
}

function diagnoseDnsEnvironment(lanAddress: string, environment: DnsEnvironment): Diagnostic[] {
  const listeners = environment.port53Listeners.split('\n').map((line) => line.trim()).filter(Boolean);
  const targets = ['*:53', '0.0.0.0:53', '[::]:53', '127.0.0.1:53', ...(lanAddress ? [`${lanAddress}:53`] : [])];
  const conflicts = listeners.filter((line) => targets.some((target) => line.includes(target)) && !/dnsmasq/i.test(line));
  const dnsmasqListening = listeners.some((line) => /dnsmasq/i.test(line) && targets.some((target) => line.includes(target)));
  const diagnostics: Diagnostic[] = [];

  if (conflicts.length) {
    const services = [...new Set(conflicts.map((line) => line.split(/\s+/)[0]).filter(Boolean))].join(', ');

    diagnostics.push({ code: 'dns-port', status: 'error', message: `DNS port 53 conflicts with: ${services}`, remedy: 'Stop or reconfigure the listed service so dnsmasq can bind 127.0.0.1 and the selected LAN address.' });
  }
  else if (dnsmasqListening) {
    diagnostics.push({ code: 'dns-port', status: 'ok', message: 'dnsmasq owns the required port 53 listeners' });
  }
  else {
    diagnostics.push({ code: 'dns-port', status: 'warning', message: 'dnsmasq is not listening on the required port 53 addresses', remedy: 'Run setup or restart the Own Domains DNS service.' });
  }

  const warpActive = /127\.0\.2\.[23]|cloudfl|warp/i.test(`${environment.dnsConfiguration}\n${environment.port53Listeners}`);

  if (warpActive) {
    const fallbackConfigured = /local domain fallback[\s\S]*?\b(?:\.own|own)\b/i.test(environment.warpSettings);

    diagnostics.push(fallbackConfigured
      ? { code: 'warp-dns', status: 'ok', message: 'WARP DNS is active with an .own Local Domain Fallback' }
      : { code: 'warp-dns', status: 'warning', message: 'WARP DNS is active but an .own Local Domain Fallback could not be confirmed', remedy: 'Configure Local Domain Fallback for own to the Mac LAN address, then rerun diagnostics.' });
  }

  return diagnostics;
}

async function inspectDnsEnvironment(): Promise<DnsEnvironment> {
  const [dnsConfiguration, port53Listeners, warpSettings] = await Promise.all([
    commandOutput('/usr/sbin/scutil', ['--dns']),
    commandOutput('/usr/sbin/lsof', ['-nP', '-iUDP:53', '-iTCP:53']),
    commandOutput('warp-cli', ['settings']),
  ]);

  return { dnsConfiguration, port53Listeners, warpSettings };
}

function commandOutput(command: string, commandArguments: string[]): Promise<string> {
  return new Promise((resolve) => {
    execFile(command, commandArguments, { timeout: 1_500 }, (error, stdout, stderr) => resolve(`${stdout}${error ? stderr : ''}`));
  });
}

function probePort(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = connect({ host: '127.0.0.1', port });

    const finish = (result: boolean) => { socket.destroy(); resolve(result) };

    socket.setTimeout(600);
    socket.once('connect', () => finish(true));
    socket.once('timeout', () => finish(false));
    socket.once('error', () => finish(false));
  });
}
