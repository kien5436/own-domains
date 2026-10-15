export type DomainMapping = {
  id: string;
  hostname: string;
  upstreamPort: number;
  tls: boolean;
  enabled: boolean;
};

export type Settings = {
  schemaVersion: 1;
  revision: number;
  lanAddress: string;
  upstreamDns: string[];
  mappings: DomainMapping[];
};

export type ApplyResult = {
  desiredRevision: number;
  appliedRevision: number;
  status: 'applied' | 'pending' | 'error';
  message?: string;
};

export type Diagnostic = {
  code: string;
  status: 'ok' | 'warning' | 'error';
  message: string;
  remedy?: string;
};

export type MappingInput = Omit<DomainMapping, 'id'> & { id?: string };
