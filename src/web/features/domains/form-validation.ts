export function normalizeDomainInput(value: string): string {
  const normalized = value.trim().toLowerCase().replace(/\.$/, '').replace(/\.own$/, '');

  return `${normalized}.own`;
}

export function isValidDomainInput(value: string): boolean {
  const hostname = normalizeDomainInput(value);

  return 253 >= hostname.length && hostname.endsWith('.own') && hostname.split('.').every((label) => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label)) && !['registry.own', 'setup.own'].some((reserved) => reserved === hostname || hostname.endsWith(`.${reserved}`));
}
