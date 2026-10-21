import type { ApplyResult, Diagnostic, MappingInput, Settings } from '../../shared/types';

export const NATIVE_PROTOCOL_VERSION = 2;

export type RuntimeStatus = 'stopped' | 'starting' | 'running' | 'degraded' | 'stopping' | 'error';

export type MutationResult = { settings: Settings; apply: ApplyResult | null };

export type StateSnapshot = { settings: Settings; runtimeStatus: RuntimeStatus; desiredRevision: number; appliedRevision: number; diagnostics: Diagnostic[]; latestEventSequence: number };

export type NativeCommand
  = | { type: 'snapshot' | 'start' | 'stop' | 'list' | 'doctor' }
    | { type: 'create'; payload: { input: MappingInput; expected_revision: number } }
    | { type: 'update'; payload: { id: string; patch: Partial<Omit<MappingInput, 'id'>>; expected_revision: number } }
    | { type: 'remove'; payload: { id: string; expected_revision: number } }
    | { type: 'updateNetwork'; payload: { lan_address: string; upstream_dns: string[]; expected_revision: number } };

export type NativeCommandResult
  = | { type: 'snapshot'; payload: StateSnapshot }
    | { type: 'settings'; payload: Settings }
    | { type: 'mutation'; payload: MutationResult }
    | { type: 'diagnostics'; payload: Diagnostic[] }
    | { type: 'runtime'; payload: RuntimeStatus }
    | { type: 'apply'; payload: ApplyResult }
    | { type: 'acknowledged' | 'subscribed' }
    | { type: 'handshake'; payload: { protocol_version: number } };

export type ProtocolError
  = | { code: 'incompatibleVersion'; supported: number; received: number }
    | { code: 'revisionConflict'; expected: number; actual: number }
    | { code: 'duplicateHostname'; hostname: string }
    | { code: 'alreadyRunning'; path: string }
    | { code: 'validation' | 'notFound' | 'runtimeUnavailable' | 'runtimeFailure' | 'storage' | 'invalidRequest'; message: string };

export type BridgeError
  = | { code: 'protocol'; error: ProtocolError }
    | { code: 'incompatible'; supported: number; received: number }
    | { code: 'unavailable' | 'transport' | 'invalidMessage'; message: string };

export type EventEnvelope = {
  protocolVersion: number;
  sequence: number;
  event:
    | { type: 'snapshot'; payload: StateSnapshot }
    | { type: 'resyncRequired'; payload: { snapshot: StateSnapshot } }
    | { type: 'settingsChanged'; payload: { revision: number } }
    | { type: 'runtimeChanged'; payload: { status: RuntimeStatus } }
    | { type: 'diagnosticsChanged'; payload: { diagnostics: Diagnostic[] } }
};

export function errorMessage(cause: unknown): string {
  if (cause instanceof Error) return cause.message;
  if (!cause || 'object' !== typeof cause) return String(cause);
  if ('error' in cause) return errorMessage(cause.error);
  if ('message' in cause) return String(cause.message);
  if ('code' in cause && 'revisionConflict' === cause.code) return 'Settings changed in another client. Reloaded the latest settings; review and try again.';
  if ('code' in cause && 'duplicateHostname' === cause.code && 'hostname' in cause) return `${cause.hostname} is already registered`;
  if ('code' in cause && ('incompatible' === cause.code || 'incompatibleVersion' === cause.code)) return 'The app and daemon use incompatible protocol versions. Update them together.';

  return 'Unable to communicate with the daemon.';
}
