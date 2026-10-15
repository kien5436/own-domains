const apply = process.argv.includes('--apply');
const steps = [
  'Install Caddy and dnsmasq with Homebrew if missing',
  'Create a dedicated dnsmasq configuration for .own',
  'Create /etc/resolver/own pointing to 127.0.0.1',
  'Install reviewed launchd services for DNS and Caddy',
  'Start the Own Domains user service on 127.0.0.1:4780',
];

process.stdout.write(`${apply ? 'Apply mode requested.' : 'Dry run; no system changes will be made.'}\n`);
for (const [index, step] of steps.entries()) process.stdout.write(`${index + 1}. ${step}\n`);

if (apply) {
  process.stderr.write('Automatic privileged setup is not enabled yet. Follow docs/setup.md and review each command.\n');
  process.exitCode = 2;
}
