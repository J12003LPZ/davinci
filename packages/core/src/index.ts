export interface SessionMetadata {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  tags: string[];
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

export interface WriterLease {
  sessionId: string;
  holderId: string;
  acquiredAt: number;
  expiresAt: number;
}

export interface RpcRequest<T = unknown> {
  id: string;
  method: string;
  params: T;
}

export interface RpcResponse<T = unknown> {
  id: string;
  result?: T;
  error?: {
    code: number;
    message: string;
    data?: unknown;
  };
}

export type AgentEvent =
  | { type: 'started'; sessionId: string; timestamp: number }
  | { type: 'message_chunk'; sessionId: string; chunk: string }
  | { type: 'tool_call_start'; sessionId: string; toolCall: ToolCall }
  | { type: 'tool_call_end'; sessionId: string; toolCallId: string; result: string }
  | { type: 'completed'; sessionId: string; timestamp: number }
  | { type: 'error'; sessionId: string; error: string };
