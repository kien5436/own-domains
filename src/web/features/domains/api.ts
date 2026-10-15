import type { ApplyResult, DomainMapping, MappingInput, Settings } from '../../../shared/types';
import { apiRequest } from '../../lib/api-client';

export type DomainSnapshot = Pick<Settings, 'revision' | 'mappings'>;

export type DomainMutationResult = { settings: DomainSnapshot; apply: ApplyResult };

export type DomainApi = {
  load: () => Promise<DomainSnapshot>;
  create: (mapping: MappingInput, expectedRevision: number) => Promise<DomainMutationResult>;
  update: (id: string, patch: Partial<Omit<MappingInput, 'id'>>, expectedRevision: number) => Promise<DomainMutationResult>;
  remove: (id: string, expectedRevision: number) => Promise<DomainMutationResult>;
};

export const domainApi: DomainApi = {
  load: () => apiRequest<Settings>('/api/domains'),
  create: (mapping, expectedRevision) => apiRequest('/api/domains', { method: 'POST', body: JSON.stringify({ mapping, expectedRevision }) }),
  update: (id, patch, expectedRevision) => apiRequest(`/api/domains/${encodeURIComponent(id)}`, { method: 'PATCH', body: JSON.stringify({ patch, expectedRevision }) }),
  remove: (id, expectedRevision) => apiRequest(`/api/domains/${encodeURIComponent(id)}`, { method: 'DELETE', body: JSON.stringify({ expectedRevision }) }),
};

export type { DomainMapping };
