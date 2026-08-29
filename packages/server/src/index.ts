import type { ClientMessage } from '@pi/core';

export interface PiServerService {
  handle(connectionId: string, message: ClientMessage): Promise<unknown>;
}

export const HANDSHAKE_TIMEOUT_MS = 5_000;
