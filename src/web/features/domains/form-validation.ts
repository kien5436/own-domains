export function normalizeDomainInput(value: string): string {
  return value.trim().toLowerCase().replace(/\.$/, '');
}

export function isValidDomainInput(value: string): boolean {
  const hostname = normalizeDomainInput(value);

  return 253 >= hostname.length && 'own' !== hostname && hostname.endsWith('.own') && hostname.split('.').every((label) => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label)) && 'setup.own' !== hostname && !hostname.endsWith('.setup.own');
}
