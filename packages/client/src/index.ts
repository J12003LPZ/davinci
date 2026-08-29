import type { ClientMessage, SessionMetadata } from '@pi/core';

export type SessionLeaseMode = 'shared' | 'exclusive';

export interface ClientTransport {
  send(message: ClientMessage): Promise<unknown>;
}

export interface SessionLease {
  id: string;
  mode: SessionLeaseMode;
  prompt(text: string): Promise<unknown>;
  detach(): Promise<void>;
}

export interface PiClient {
  connect(): Promise<unknown>;
  listSessions(): Promise<SessionMetadata[]>;
  createSession(cwd?: string, name?: string): Promise<SessionLease>;
  attachSession(sessionId: string): Promise<SessionLease>;
}
