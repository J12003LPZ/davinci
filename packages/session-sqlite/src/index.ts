import type { SessionMetadata, WriterLease, WriterLeaseOptions } from '@pi/core';

export type { SessionMetadata, WriterLease, WriterLeaseOptions };

export const DEFAULT_WRITER_LEASE: WriterLeaseOptions = {
  ttlMs: 30_000,
  heartbeatIntervalMs: 10_000,
};

export function validateWriterLeaseOptions(options: WriterLeaseOptions): void {
  if (options.ttlMs <= 0) {
    throw new RangeError('writerLease.ttlMs must be positive');
  }
  if (options.heartbeatIntervalMs <= 0 || options.heartbeatIntervalMs >= options.ttlMs) {
    throw new RangeError('writerLease.heartbeatIntervalMs must be positive and less than ttlMs');
  }
}

interface LeaseRow {
  ownerId: string;
  fence: number;
  expiresAtMs: number;
}

/**
 * In-memory stand-in that preserves official acquire/renew/release/fence SQL semantics
 * so TypeScript remains the fixture authority until Phase 8.
 */
export class InMemorySessionStore {
  private leases = new Map<string, LeaseRow>();
  private sessions = new Map<string, SessionMetadata>();

  async createSession(id: string, cwd: string, metadata?: Record<string, unknown>): Promise<SessionMetadata> {
    const now = Date.now();
    const meta: SessionMetadata = { id, createdAt: now, cwd, metadata };
    this.sessions.set(id, meta);
    return meta;
  }

  async listSessions(cwd?: string): Promise<SessionMetadata[]> {
    return [...this.sessions.values()].filter((s) => !cwd || s.cwd === cwd);
  }

  acquireWriterLease(sessionId: string, ownerId: string, now: number, expiresAtMs: number): WriterLease | undefined {
    const existing = this.leases.get(sessionId);
    if (!existing) {
      const lease = { ownerId, fence: 1, expiresAtMs };
      this.leases.set(sessionId, lease);
      return lease;
    }
    if (existing.expiresAtMs <= now) {
      const lease = { ownerId, fence: existing.fence + 1, expiresAtMs };
      this.leases.set(sessionId, lease);
      return lease;
    }
    return undefined;
  }

  renewWriterLease(sessionId: string, lease: WriterLease, now: number, expiresAtMs: number): boolean {
    const existing = this.leases.get(sessionId);
    if (
      !existing ||
      existing.ownerId !== lease.ownerId ||
      existing.fence !== lease.fence ||
      existing.expiresAtMs <= now
    ) {
      return false;
    }
    existing.expiresAtMs = expiresAtMs;
    lease.expiresAtMs = expiresAtMs;
    return true;
  }

  releaseWriterLease(sessionId: string, lease: WriterLease): void {
    const existing = this.leases.get(sessionId);
    if (existing && existing.ownerId === lease.ownerId && existing.fence === lease.fence) {
      this.leases.delete(sessionId);
    }
  }

  deleteWriterLease(sessionId: string): void {
    this.leases.delete(sessionId);
  }

  getCurrentLease(sessionId: string): WriterLease | undefined {
    return this.leases.get(sessionId);
  }
}
