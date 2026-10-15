import { createHash } from 'node:crypto';
import { join } from 'node:path';

import { buildDnsConfig } from '../server/network/dns-config';

export type SetupArtifact = {
  path: string;
  content: string;
  mode: number;
  backupExisting: boolean;
};

export type SetupPlan = { artifacts: SetupArtifact[] };

type SetupPlanOptions = {
  backupDirectory: string;
  brewPrefix: string;
  homeDirectory: string;
  repositoryRoot: string;
  userName: string;
  lanAddress: string;
  upstreamDns: string[];
};

export function buildSetupPlan(options: SetupPlanOptions): SetupPlan {
  const { backupDirectory, brewPrefix, homeDirectory, repositoryRoot, userName, lanAddress, upstreamDns } = options;
  const applicationDirectory = join(homeDirectory, 'Library/Application Support/OwnDomains');
  const dnsmasqConfig = join(brewPrefix, 'etc/dnsmasq.d/own-domains.conf');
  const caddyConfig = join(applicationDirectory, 'caddy-bootstrap.json');
  const artifacts: SetupArtifact[] = [
    artifact('/etc/resolver/own', 'nameserver 127.0.0.1\n', 0o644),
    artifact(dnsmasqConfig, buildDnsConfig({ schemaVersion: 1, revision: 0, lanAddress, upstreamDns, mappings: [] }), 0o644),
    artifact(caddyConfig, `${JSON.stringify({ admin: { listen: 'unix//tmp/own-domains-caddy.sock' } }, null, 2)}\n`, 0o644),
    artifact('/Library/LaunchDaemons/dev.owndomains.dnsmasq.plist', launchdPlist('dev.owndomains.dnsmasq', [join(brewPrefix, 'sbin/dnsmasq'), '--keep-in-foreground', `--conf-file=${dnsmasqConfig}`], undefined, true), 0o644),
    artifact('/Library/LaunchDaemons/dev.owndomains.caddy.plist', launchdPlist('dev.owndomains.caddy', [join(brewPrefix, 'bin/caddy'), 'run', '--config', caddyConfig], undefined, true), 0o644),
    artifact(join(homeDirectory, 'Library/LaunchAgents/dev.owndomains.backend.plist'), launchdPlist('dev.owndomains.backend', [join(brewPrefix, 'bin/pnpm'), 'exec', 'tsx', 'src/server/index.ts'], repositoryRoot, true), 0o644),
  ];
  const manifestPath = join(applicationDirectory, 'install-manifest.json');
  const manifest = {
    schemaVersion: 1,
    userName,
    backupDirectory,
    artifacts: artifacts.map((item) => ({ path: item.path, sha256: createHash('sha256').update(item.content).digest('hex') })),
  };

  artifacts.push({ path: manifestPath, content: `${JSON.stringify(manifest, null, 2)}\n`, mode: 0o600, backupExisting: false });

  return { artifacts };
}

function artifact(path: string, content: string, mode: number): SetupArtifact {
  return { path, content, mode, backupExisting: true };
}

function launchdPlist(label: string, programArguments: string[], workingDirectory?: string, runAtLoad = false): string {
  const argumentsXml = programArguments.map((argument) => `      <string>${escapeXml(argument)}</string>`).join('\n');
  const workingDirectoryXml = workingDirectory ? `\n    <key>WorkingDirectory</key>\n    <string>${escapeXml(workingDirectory)}</string>` : '';

  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>${escapeXml(label)}</string>
    <key>ProgramArguments</key>
    <array>
${argumentsXml}
    </array>${workingDirectoryXml}
    <key>RunAtLoad</key>
    <${runAtLoad}/>
    <key>KeepAlive</key>
    <true/>
  </dict>
</plist>
`;
}

function escapeXml(value: string): string {
  return value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;').replaceAll("'", '&apos;');
}
