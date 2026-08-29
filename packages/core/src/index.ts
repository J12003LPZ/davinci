export const PROTOCOL_VERSION = 1 as const;

export interface WriterLease {
  ownerId: string;
  fence: number;
  expiresAtMs: number;
}

export interface WriterLeaseOptions {
  ttlMs: number;
  heartbeatIntervalMs: number;
}

export type SessionErrorCode =
  | 'not_found'
  | 'already_exists'
  | 'invalid_entry'
  | 'invalid_payload'
  | 'invalid_lane'
  | 'invalid_query'
  | 'invalid_fork_target'
  | 'storage';

export interface SessionMetadata {
  id: string;
  createdAt: number;
  updatedAt?: number;
  parentSessionId?: string;
  sessionName?: string;
  cwd?: string;
  path?: string;
  metadata?: Record<string, unknown>;
}

export type Role = 'system' | 'user' | 'assistant' | 'tool';

export interface ToolCall {
  id: string;
  name: string;
  arguments: string;
}

export interface Message {
  id: string;
  sessionId: string;
  role: Role;
  content: string;
  toolCalls?: ToolCall[];
  toolCallId?: string;
  timestamp: number;
}

export type ThinkingLevel = 'off' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' | 'max';

export type ProtocolErrorCode =
  | 'version'
  | 'busy'
  | 'session_locked'
  | 'not_found'
  | 'invalid_request'
  | 'not_implemented'
  | 'internal_error';

export interface ProtocolError {
  code: ProtocolErrorCode;
  message: string;
  details?: unknown;
}

export type ClientMessage =
  | { type: 'hello'; version: number }
  | { type: 'request'; id: string; request: Command };

export type Command =
  | { command: 'list' }
  | { command: 'create'; cwd?: string; name?: string }
  | { command: 'attach'; sessionId: string }
  | { command: 'detach'; sessionId: string }
  | { command: 'prompt'; sessionId: string; text: string }
  | { command: 'steer'; sessionId: string; text: string }
  | { command: 'abort'; sessionId: string }
  | { command: 'set_model'; sessionId: string; model: { provider: string; id: string } }
  | { command: 'set_thinking'; sessionId: string; thinkingLevel: ThinkingLevel };
