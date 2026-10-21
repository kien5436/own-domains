import { describe, expect, it, vi } from 'vitest';
import { RuntimeStore } from '../../../../src/web/features/runtime/runtime-store';
import type { NativeClient } from '../../../../src/web/lib/native-client';
import type { EventEnvelope, StateSnapshot } from '../../../../src/web/lib/native-protocol';
import type { ApplyResult } from '../../../../src/shared/types';

export const snapshot = (revision = 0, sequence = revision): StateSnapshot => ({ settings: { schemaVersion: 1, revision, lanAddress: '', upstreamDns: ['1.1.1.1'], mappings: [] }, runtimeStatus: 'stopped', desiredRevision: revision, appliedRevision: 0, diagnostics: [], latestEventSequence: sequence });

function harness() {
  let state = snapshot();
  let mutationApply: ApplyResult | null = null;

  let emit: (event: EventEnvelope) => void = () => { throw new Error('Not subscribed') };

  let fail: (error: unknown) => void = () => { throw new Error('Not subscribed') };

  const unsubscribe = vi.fn(() => Promise.resolve());
  const client: NativeClient = {
    request: vi.fn<NativeClient['request']>((command) => {
      if ('snapshot' === command.type) return Promise.resolve({ type: 'snapshot', payload: state });
      const revision = state.settings.revision + 1;

      state = {
        ...state,
        settings: { ...state.settings, revision },
        desiredRevision: revision,
        appliedRevision: mutationApply?.appliedRevision ?? state.appliedRevision,
        runtimeStatus: mutationApply ? ('error' === mutationApply.status ? 'degraded' : 'running') : state.runtimeStatus,
        latestEventSequence: state.latestEventSequence + 1,
      };

      return Promise.resolve({ type: 'mutation', payload: { settings: state.settings, apply: mutationApply } });
    }),
    subscribe: vi.fn((after, onEvent, onDisconnect) => {
      expect(after).toBeNull();
      emit = onEvent; fail = onDisconnect;

      return Promise.resolve(unsubscribe);
    }),
  };

  return { client, unsubscribe, set(value: StateSnapshot) { state = value }, apply(value: ApplyResult | null) { mutationApply = value }, emit: (sequence: number) => emit({ protocolVersion: 2, sequence, event: { type: 'settingsChanged', payload: { revision: sequence } } }), fail: () => fail(new Error('disconnected')) };
}

describe('RuntimeStore', () => {
  it('subscribes before reading and updates all consumers after CLI mutations', async () => {
    const h = harness(); const store = new RuntimeStore(h.client);

    await store.connect(); expect(store.getSnapshot().snapshot?.runtimeStatus).toBe('stopped');
    expect(vi.mocked(h.client.subscribe).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(h.client.request).mock.invocationCallOrder[0]);
    h.set(snapshot(2)); h.emit(2);
    await vi.waitFor(() => expect(store.getSnapshot().snapshot?.settings.revision).toBe(2));
    await store.disconnect(); expect(h.unsubscribe).toHaveBeenCalledOnce();
  });
  it('reloads conflicts without retrying writes and retains the error', async () => {
    const h = harness(); const store = new RuntimeStore(h.client);

    await store.connect();
    vi.mocked(h.client.request).mockRejectedValueOnce({ code: 'protocol', error: { code: 'revisionConflict', expected: 0, actual: 3 } });
    h.set(snapshot(3));
    await expect(store.mutate({ type: 'remove', payload: { id: 'a', expected_revision: 0 } })).rejects.toBeDefined();
    expect(store.getSnapshot().snapshot?.settings.revision).toBe(3);
    expect(store.getSnapshot().error).toBeTruthy();
    expect(vi.mocked(h.client.request).mock.calls.filter(([command]) => 'remove' === command.type)).toHaveLength(1);
    await store.disconnect();
  });
  it('keeps saved degraded settings visible and clears the apply error after recovery', async () => {
    const h = harness(); const store = new RuntimeStore(h.client);

    h.set({ ...snapshot(), runtimeStatus: 'running' });
    await store.connect();
    h.apply({ desiredRevision: 1, appliedRevision: 0, status: 'error', message: 'reload failed' });
    await store.mutate({ type: 'create', payload: { input: { hostname: 'shop.own', upstreamPort: 3000, tls: false, enabled: true }, expected_revision: 0 } });
    expect(store.getSnapshot()).toMatchObject({ error: 'reload failed', snapshot: { runtimeStatus: 'degraded', desiredRevision: 1, appliedRevision: 0, settings: { revision: 1 } } });

    h.apply({ desiredRevision: 2, appliedRevision: 2, status: 'applied' });
    await store.mutate({ type: 'updateNetwork', payload: { lan_address: '192.168.1.20', upstream_dns: ['1.1.1.1'], expected_revision: 1 } });
    expect(store.getSnapshot()).toMatchObject({ error: '', snapshot: { runtimeStatus: 'running', desiredRevision: 2, appliedRevision: 2, settings: { revision: 2 } } });
    await store.disconnect();
  });
  it('ignores delayed responses from the previous connection generation', async () => {
    const h = harness(); const store = new RuntimeStore(h.client);
    let resolve!: (value: { type: 'snapshot'; payload: StateSnapshot }) => void;

    vi.mocked(h.client.request).mockImplementationOnce(() => new Promise((done) => { resolve = done }));
    const first = store.connect();

    await vi.waitFor(() => expect(resolve).toBeDefined());
    await store.disconnect(); h.set(snapshot(0)); await store.connect();
    resolve({ type: 'snapshot', payload: snapshot(99) }); await first;
    expect(store.getSnapshot().snapshot?.settings.revision).toBe(0);
    await store.disconnect();
  });
  it('does not finish connecting after an immediate disconnect', async () => {
    const h = harness(); const store = new RuntimeStore(h.client);
    const staleUnsubscribe = vi.fn(() => Promise.resolve());

    vi.mocked(h.client.subscribe).mockResolvedValueOnce(staleUnsubscribe);
    const connecting = store.connect();

    await store.disconnect();
    await connecting;

    expect(store.getSnapshot().connection).toBe('disconnected');
    expect(vi.mocked(h.client.subscribe)).not.toHaveBeenCalled();
    expect(staleUnsubscribe).not.toHaveBeenCalled();
    expect(vi.mocked(h.client.request)).not.toHaveBeenCalled();
  });
  it('keeps only the newest connection when subscriptions finish out of order', async () => {
    const h = harness(); const store = new RuntimeStore(h.client);
    const finishes: ((unsubscribe: () => Promise<void>) => void)[] = [];
    const oldUnsubscribe = vi.fn(() => Promise.resolve());
    const currentUnsubscribe = vi.fn(() => Promise.resolve());

    vi.mocked(h.client.subscribe).mockImplementation(() => new Promise((resolve) => { finishes.push(resolve) }));
    const oldConnection = store.connect();

    await vi.waitFor(() => expect(finishes).toHaveLength(1));
    const currentConnection = store.connect();

    await vi.waitFor(() => expect(finishes).toHaveLength(2));
    finishes[1](currentUnsubscribe);
    await currentConnection;
    finishes[0](oldUnsubscribe);
    await oldConnection;

    expect(store.getSnapshot().connection).toBe('connected');
    expect(oldUnsubscribe).toHaveBeenCalledOnce();
    expect(currentUnsubscribe).not.toHaveBeenCalled();
    await store.disconnect();
    expect(currentUnsubscribe).toHaveBeenCalledOnce();
  });
  it('coalesces notifications arriving during a snapshot read and ignores duplicate events', async () => {
    const h = harness(); const store = new RuntimeStore(h.client);

    await store.connect();
    let resolve!: (value: { type: 'snapshot'; payload: StateSnapshot }) => void;

    vi.mocked(h.client.request).mockImplementationOnce(() => new Promise((done) => { resolve = done }));
    h.emit(1); h.set(snapshot(2)); h.emit(2);
    resolve({ type: 'snapshot', payload: snapshot(1) });
    await vi.waitFor(() => expect(store.getSnapshot().snapshot?.settings.revision).toBe(2));
    const count = vi.mocked(h.client.request).mock.calls.length;

    h.emit(2);
    expect(vi.mocked(h.client.request).mock.calls).toHaveLength(count);
    await store.disconnect();
  });
  it('disconnects without starting the daemon and rejects mutations while disconnected', async () => {
    const h = harness(); const store = new RuntimeStore(h.client);

    await store.connect(); h.fail();
    expect(store.getSnapshot().connection).toBe('disconnected');
    await expect(store.mutate({ type: 'remove', payload: { id: 'a', expected_revision: 0 } })).rejects.toBeDefined();
    expect(vi.mocked(h.client.request).mock.calls.some(([command]) => 'start' === command.type)).toBe(false);
    await store.disconnect();
  });
});
