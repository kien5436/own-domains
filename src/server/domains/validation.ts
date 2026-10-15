import { randomUUID } from 'node:crypto';

import type { DomainMapping, MappingInput } from '../../shared/types';

const RESERVED_PORTS = new Set([53, 80, 443, 4780]);
const LABEL_PATTERN = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/;

export class ValidationError extends Error {}

export function normalizeHostname(input: string): string {
  return input.toLowerCase().replace(/\.$/, '');
}

export function validateHostname(input: unknown): string {
  if ('string' !== typeof input) throw new ValidationError('Domain must be text');
  const hostname = normalizeHostname(input.trim());

  if (253 < hostname.length || !hostname.endsWith('.own') || 'own' === hostname) throw new ValidationError('Enter a valid .own domain');
  if ('setup.own' === hostname || hostname.endsWith('.setup.own')) throw new ValidationError('setup.own is reserved');
  if (!hostname.split('.').every((label) => LABEL_PATTERN.test(label))) throw new ValidationError('Enter a valid .own domain');

  return hostname;
}

export function validateMapping(input: unknown): DomainMapping {
  if (!input || 'object' !== typeof input) throw new ValidationError('Mapping is required');
  const candidate = input as Partial<MappingInput>;
  const hostname = validateHostname(candidate.hostname);

  if (!Number.isInteger(candidate.upstreamPort) || 1 > candidate.upstreamPort! || 65535 < candidate.upstreamPort! || RESERVED_PORTS.has(candidate.upstreamPort!))
    throw new ValidationError('Choose an available local port');

  if ('boolean' !== typeof candidate.tls || 'boolean' !== typeof candidate.enabled) throw new ValidationError('TLS and enabled must be boolean');

  return {
    id: 'string' === typeof candidate.id && candidate.id ? candidate.id : randomUUID(),
    hostname,
    upstreamPort: candidate.upstreamPort!,
    tls: candidate.tls,
    enabled: candidate.enabled,
  };
}
