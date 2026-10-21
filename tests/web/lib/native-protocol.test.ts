import { expect, it } from 'vitest';
import contract from '../../fixtures/ipc/desktop-contract.json';
import { errorMessage, type NativeCommand, type NativeCommandResult } from '../../../src/web/lib/native-protocol';

it('sends exactly the Rust version-2 command payloads, including snake_case revision fields', () => {
  const commands: NativeCommand[] = [
    { type: 'snapshot' },
    { type: 'start' },
    { type: 'stop' },
    { type: 'list' },
    { type: 'doctor' },
    { type: 'create', payload: { input: { id: 'one', hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true }, expected_revision: 0 } },
    { type: 'update', payload: { id: 'one', patch: { hostname: 'shop.own', upstreamPort: 4000, tls: true, enabled: false }, expected_revision: 1 } },
    { type: 'remove', payload: { id: 'one', expected_revision: 2 } },
    { type: 'updateNetwork', payload: { lan_address: '192.168.1.20', upstream_dns: ['1.1.1.1'], expected_revision: 3 } },
  ];

  expect(commands).toEqual(contract.commands);
  expect(contract.subscription).toEqual({ type: 'subscribe', payload: { after_sequence: 4 } });
});
it('parses offline and degraded mutation results from the Rust contract', () => {
  const offline = contract.results[2] as NativeCommandResult;
  const degraded = contract.results[3] as NativeCommandResult;

  expect(offline).toMatchObject({ type: 'mutation', payload: { apply: null } });
  expect(degraded).toMatchObject({ type: 'mutation', payload: { apply: { status: 'error', appliedRevision: 0, desiredRevision: 1 } } });
  expect(contract.events[3]).toMatchObject({ protocolVersion: 2, event: { type: 'runtimeChanged', payload: { status: 'degraded' } } });
  expect(contract.errors[0]).toEqual({ code: 'incompatibleVersion', supported: 2, received: 1 });
});
it('shows actionable messages for Rust protocol errors without losing the conflict distinction', () => {
  const messages = contract.errors.map((error) => errorMessage({ code: 'protocol', error }));

  expect(messages[0]).toContain('incompatible');
  expect(messages[1]).toContain('another client');
  expect(messages[2]).toBe('shop.own is already registered');
  expect(messages[6]).toBe('unavailable');
});
