import { SessionMetadata, Message, WriterLease } from '@pi/core';

export interface ISessionStore {
  createSession(id: string, title: string, tags?: string[]): Promise<SessionMetadata>;
  getSession(id: string): Promise<SessionMetadata | null>;
  listSessions(): Promise<SessionMetadata[]>;
  deleteSession(id: string): Promise<boolean>;

  appendMessage(message: Message): Promise<void>;
  getMessages(sessionId: string): Promise<Message[]>;

  acquireWriterLease(sessionId: string, holderId: string, ttlMs: number): Promise<boolean>;
  renewWriterLease(sessionId: string, holderId: string, ttlMs: number): Promise<boolean>;
  releaseWriterLease(sessionId: string, holderId: string): Promise<boolean>;
  getCurrentLease(sessionId: string): Promise<WriterLease | null>;
}

export class InMemorySessionStore implements ISessionStore {
  private sessions: Map<string, SessionMetadata> = new Map();
  private messages: Map<string, Message[]> = new Map();
  private leases: Map<string, WriterLease> = new Map();

  async createSession(id: string, title: string, tags: string[] = []): Promise<SessionMetadata> {
    const now = Date.now();
    const meta: SessionMetadata = { id, title, createdAt: now, updatedAt: now, tags };
    this.sessions.set(id, meta);
    this.messages.set(id, []);
    return meta;
  }

  async getSession(id: string): Promise<SessionMetadata | null> {
    return this.sessions.get(id) || null;
  }

  async listSessions(): Promise<SessionMetadata[]> {
    return Array.from(this.sessions.values()).sort((a, b) => b.updatedAt - a.updatedAt);
  }

  async deleteSession(id: string): Promise<boolean> {
    this.leases.delete(id);
    this.messages.delete(id);
    return this.sessions.delete(id);
  }

  async appendMessage(message: Message): Promise<void> {
    const list = this.messages.get(message.sessionId) || [];
    list.push(message);
    this.messages.set(message.sessionId, list);

    const session = this.sessions.get(message.sessionId);
    if (session) {
      session.updatedAt = message.timestamp;
    }
  }

  async getMessages(sessionId: string): Promise<Message[]> {
    return this.messages.get(sessionId) || [];
  }

  async acquireWriterLease(sessionId: string, holderId: string, ttlMs: number): Promise<boolean> {
    const now = Date.now();
    const existing = this.leases.get(sessionId);

    if (existing && existing.expiresAt > now && existing.holderId !== holderId) {
      return false;
    }

    this.leases.set(sessionId, {
      sessionId,
      holderId,
      acquiredAt: now,
      expiresAt: now + ttlMs,
    });
    return true;
  }

  async renewWriterLease(sessionId: string, holderId: string, ttlMs: number): Promise<boolean> {
    const now = Date.now();
    const existing = this.leases.get(sessionId);

    if (!existing || existing.holderId !== holderId || existing.expiresAt <= now) {
      return false;
    }

    existing.expiresAt = now + ttlMs;
    return true;
  }

  async releaseWriterLease(sessionId: string, holderId: string): Promise<boolean> {
    const existing = this.leases.get(sessionId);
    if (!existing || existing.holderId !== holderId) {
      return false;
    }
    this.leases.delete(sessionId);
    return true;
  }

  async getCurrentLease(sessionId: string): Promise<WriterLease | null> {
    const now = Date.now();
    const existing = this.leases.get(sessionId);
    if (existing && existing.expiresAt > now) {
      return existing;
    }
    return null;
  }
}
