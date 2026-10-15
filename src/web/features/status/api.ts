import type { Diagnostic } from '../../../shared/types';
import { apiRequest } from '../../lib/api-client';

export type StatusResponse = { lanAddress: string; desiredRevision: number; appliedRevision: number; applyStatus: 'applied' | 'pending'; diagnostics: Diagnostic[] };

export function loadStatus(): Promise<StatusResponse> {
  return apiRequest('/api/status');
}
