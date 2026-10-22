import type { NativeClient } from '../../lib/native-client';
import { errorMessage, type EventEnvelope, type NativeCommand, type StateSnapshot } from '../../lib/native-protocol';

export type RuntimeState = { snapshot: StateSnapshot | null; connection: 'connecting' | 'connected' | 'disconnected'; busy: boolean; error: string };

export class RuntimeStore {
  private state: RuntimeState = { snapshot: null, connection: 'disconnected', busy: false, error: '' };
  private listeners = new Set<() => void>();
  private generation = 0;
  private sequence = -1;
  private unsubscribe?: () => Promise<void>;
  private reading?: Promise<void>;
  private dirty = false;
  private reconnect?: ReturnType<typeof setTimeout>;

  constructor(private client: NativeClient) {}
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);

    return () => { this.listeners.delete(listener) };
  };
  private publish(patch: Partial<RuntimeState>) { this.state = { ...this.state, ...patch }; for (const listener of this.listeners) listener(); }

  private async stopSubscription() {
    const unsubscribe = this.unsubscribe;

    this.unsubscribe = undefined;
    await unsubscribe?.();
  }

  async disconnect() {
    this.generation++;
    clearTimeout(this.reconnect);
    this.reading = undefined;
    this.dirty = false;
    this.publish({ connection: 'disconnected', busy: false, error: '' });
    await this.stopSubscription();
  }

  async connect() {
    const generation = ++this.generation;

    clearTimeout(this.reconnect);
    await this.stopSubscription();
    if (generation !== this.generation) return;

    this.reading = undefined;
    this.dirty = false;
    this.sequence = -1;
    this.publish({ snapshot: null, connection: 'connecting', error: '' });

    try {
      const unsubscribe = await this.client.subscribe(null, (event) => this.event(event, generation), (error) => this.failed(error, generation));

      if (generation !== this.generation) {
        await unsubscribe();

        return;
      }

      this.unsubscribe = unsubscribe;
      await this.refresh(generation);
      if (generation === this.generation) this.publish({ connection: 'connected' });
    }
    catch (error) { this.failed(error, generation) }
  }

  private failed(error: unknown, generation: number) {
    if (generation !== this.generation) return;
    const next = ++this.generation;

    this.reading = undefined;
    this.publish({ connection: 'disconnected', busy: false, error: errorMessage(error) });
    const unsubscribe = this.unsubscribe;

    this.unsubscribe = undefined;
    void unsubscribe?.().catch(() => { /* Disconnection already recorded; cleanup cannot restore the peer. */ });
    clearTimeout(this.reconnect);
    this.reconnect = setTimeout(() => { if (next === this.generation) void this.connect(); }, 1000);
  }

  private event(event: EventEnvelope, generation: number) {
    if (generation !== this.generation || event.sequence <= this.sequence) return;
    this.sequence = event.sequence;
    // Read after subscribing: the broker snapshot may precede its registration.
    void this.refresh(generation).catch((error) => this.failed(error, generation));
  }

  private refresh(generation = this.generation): Promise<void> {
    if (this.reading) {
      this.dirty = true;

      return this.reading;
    }

    const read = async () => {
      do {
        this.dirty = false;
        // Serialize snapshot reads so an older response cannot overwrite a newer one.
        // eslint-disable-next-line no-await-in-loop
        const result = await this.client.request({ type: 'snapshot' });

        if (generation !== this.generation) return;
        if ('snapshot' !== result.type) throw new Error('Daemon returned an invalid snapshot');
        const previous = this.state.snapshot;

        if (!previous || result.payload.latestEventSequence >= previous.latestEventSequence) this.publish({ snapshot: result.payload });
        this.sequence = Math.max(this.sequence, result.payload.latestEventSequence);
      } while (this.dirty);
    };

    const pending = read().finally(() => { if (generation === this.generation) this.reading = undefined; });

    this.reading = pending;

    return pending;
  }

  async mutate(command: NativeCommand): Promise<void> {
    if ('connected' !== this.state.connection || this.state.busy) throw new Error('Wait for the daemon connection before changing settings.');
    const generation = this.generation;

    this.publish({ busy: true, error: '' });

    try {
      const result = await this.client.request(command);

      if ('mutation' !== result.type) throw new Error('Daemon returned an invalid mutation result');

      if (generation === this.generation) {
        if ('error' === result.payload.apply?.status) this.publish({ error: result.payload.apply.message ?? 'Saved, but the runtime could not apply the configuration.' });
        await this.refresh(generation);
      }
    }
    catch (error) {
      if (generation === this.generation) {
        this.publish({ error: errorMessage(error) });

        try { await this.refresh(generation) }
        catch (reloadError) { this.failed(reloadError, generation) }
      }

      throw new Error(errorMessage(error), { cause: error });
    }
    finally { if (generation === this.generation) this.publish({ busy: false }); }
  }
}
